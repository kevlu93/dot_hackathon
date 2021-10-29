use crate::utils::StatefulList;
use std::collections::{BTreeMap, HashMap};
use chrono::{
    Date, 
    DateTime,
    offset::Utc};
use std::error::Error;
use ::reqwest::blocking::Client;
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};

#[derive(Debug)]
struct Transaction {
    amount: f64,
    timestamp: chrono::DateTime<Utc>, 
}

#[derive(Debug)]
struct DailyLiquidityFlow{
    token0: String,
    token1: String,
    token0amount: f64,
    token1amount: f64,
    date: chrono::Date<Utc>,
}

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/schema.json",
    query_path = "src/transfers_query.graphql",
    response_derives = "Debug",
)]

pub struct TransfersQuery;

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/schema.json",
    query_path = "src/price_query.graphql",
    response_derives = "Debug",
)]

pub struct PriceQuery;

pub struct App {
    pub selected_tab: usize,
    pub wallet_list_state:  StatefulList<String>,
    pub selected_token: String,
    pub total_holdings: BTreeMap<String, f64>,
    pub total_holdings_mv: HashMap<String, f64>,
    pub mwr_map: HashMap<String, f64>,
    pub daily_holdings_mv: BTreeMap<String, BTreeMap<Date<Utc>, f64>>,
    pub cost_bases: HashMap<String, Vec<(Date<Utc>, f64)>>,
    pub base_token: String,
}


impl App {
    pub fn new(account: String) -> App {
        let transactions = perform_query(&account).expect("Query failed for account id");
        let mut daily_holdings = BTreeMap::new();
        let mut total_holdings = BTreeMap::new();
        let mut total_holdings_mv = HashMap::new();
        let mut mwr_map = HashMap::new();
        let mut daily_holdings_mv = BTreeMap::new();
        let mut cost_bases = HashMap::new();
        let base_token = "AUSD";
        //get data from subquery
        generate_wallet_data(&transactions, &mut daily_holdings, &mut total_holdings, &mut total_holdings_mv, &mut mwr_map, &mut daily_holdings_mv, &mut cost_bases, &base_token);
        //get list of tokens
        let wallet_tokens = total_holdings.keys().map(|s| s.to_string()).collect();
        let mut wallet_list_state = StatefulList::with_items(wallet_tokens);
        wallet_list_state.state.select(Some(0));
        let selected_token = wallet_list_state.items.get(wallet_list_state.state.selected().unwrap()).unwrap().to_string();
        //create tui app
        App {
           selected_tab : 0,
           wallet_list_state: wallet_list_state,
           selected_token: selected_token,
           total_holdings: total_holdings,
           total_holdings_mv: total_holdings_mv,
           mwr_map: mwr_map,
           daily_holdings_mv: daily_holdings_mv,
           cost_bases: cost_bases,
           base_token: base_token.to_string(),
        }
    }
}

fn perform_query(account_id: &String) -> Result<BTreeMap<String, Vec<Transaction>>, Box<dyn Error>> {
    let variables = transfers_query::Variables{
        accountid: account_id.to_string(),
    };

    let client = Client::new();

    let response_body = post_graphql::<TransfersQuery, _>(&client, "https://api.subquery.network/sq/pparrott/prices-and-daily-liquidity-pool", variables).unwrap();
    
    let response_data = &response_body.data.expect("missing response data");

    let mut currency_map = BTreeMap::new();

    //We are only looking at one account, so accounts will only have one account in its nodes vector
    if let Some(n) = &response_data.accounts.as_ref().expect("No transfers for account").nodes.iter().flat_map(|n| n.iter()).next() {
        for c in n.balances.nodes.iter().flat_map(|n| n.iter()) {
            //Now we want to iterate through the transactions made by the account
            let mut transactions = vec![];
            for t in c.transfers.nodes.iter().flat_map(|n| n.iter()) {
                transactions.push(
                    Transaction{
                        //Amount from API needs to be multiplied by factor of 10^-13
                        amount: t.amount.parse::<f64>().expect("Unable to parse transaction amount") * 10.0_f64.powi(-13),
                        timestamp: chrono::DateTime::from_utc(
                            //we receive Unix timestamp from Typescript, which is in milliseconds. chrono expects it in seconds instead, so need to divid by 1000
                            chrono::NaiveDateTime::from_timestamp(t.date.parse::<i64>().expect("Unable to parse integer representation of date")/1000, 0),
                            Utc),
                    }
                )
            }
            currency_map.insert(c.currency.clone(), transactions);
        }
    } 

    Ok(currency_map)
}

fn perform_price_query() -> Result<BTreeMap<(String, String), Vec<(chrono::Date<Utc>, (f64, f64))>>, Box<dyn Error>> {
    let variables = price_query::Variables{
    };

    let client = Client::new();

    let response_body = post_graphql::<PriceQuery, _>(&client, "https://api.subquery.network/sq/pparrott/prices-and-daily-liquidity-pool", variables).unwrap();
    
    let response_data = &response_body.data.expect("missing response data");
    
    let mut flows = vec![];
    for f in response_data.liquidity_daily_summaries.as_ref().expect("There should be at least one liquidity pool").nodes.iter().flat_map(|n| n.iter()) {
        flows.push(
            DailyLiquidityFlow {
                token0: f.token0.clone(),
                token1: f.token1.clone(),
                token0amount: f.token0_daily_total.parse::<f64>().expect("Error parsing liquidity flow amount"),
                token1amount: f.token1_daily_total.parse::<f64>().expect("Error parsing liquidity flow amount"),
                date: chrono::Date::from_utc(chrono::NaiveDate::parse_from_str(&f.date, "%Y%m%d").expect("Unable to parse string representation of date"),Utc)
            }
        )
    }

    let mut currency_map: BTreeMap<(String, String), Vec<(chrono::Date<Utc>, (f64, f64))>> = BTreeMap::new();
    for f in flows {
        let token_combo = (f.token0, f.token1);
        if let Some(p) = currency_map.get_mut(&token_combo) {
            p.push((f.date, (f.token0amount, f.token1amount)));
        } else {
            currency_map.insert(token_combo.clone(), vec![(f.date, (f.token0amount, f.token1amount))]);
        }
    }

    let mut final_map = BTreeMap::new();
    for (k, v) in currency_map {
        let mut start_date = v.iter().next().expect("Pool must have at least one daily flow in order to have been grabbed").0; 
        let end_date = v.iter().last().expect("Pool must have at least one daily flow in order to have been grabbed").0;
        let mut dates = vec![];
        while start_date <= end_date {
            dates.push(start_date);
            start_date = start_date + chrono::Duration::days(1);
        }
        //Now generate a column where each date has the net amount transacted on that day
        let date_iter = dates.iter();
        let flow_iter = v.iter();
        let mut flows_map = BTreeMap::new();
        for d in date_iter {
            flows_map.insert(*d, (0.0, 0.0));
        }  
        for t in flow_iter {
            flows_map.insert(t.0, t.1);
        }
        //Now create a column that will calculate the running total in the account
        let running_total: Vec<(f64, f64)> = flows_map.values().scan((0.0, 0.0), |state, x| {*state = (state.0 + x.0,state.1 + x.1); Some(*state)}).collect();
        let running_total_dates: Vec<(chrono::Date<Utc>, (f64, f64))> = flows_map.keys().zip(running_total.iter()).map(|(d, a)| (*d, *a)).collect();
        final_map.insert(k, running_total_dates);
    }

    Ok(final_map)
}

fn generate_wallet_data(
    transactions: &BTreeMap<String, Vec<Transaction>>, 
    daily_holdings: &mut BTreeMap<String, BTreeMap<chrono::Date<Utc>, f64>>,
    total_holdings: &mut BTreeMap<String, f64>,
    total_holdings_mv: &mut HashMap<String, f64>,
    mwr_map: &mut HashMap<String, f64>,
    daily_holdings_mv: &mut BTreeMap<String, BTreeMap<Date<Utc>, f64>>,
    cost_bases: &mut HashMap<String, Vec<(Date<Utc>, f64)>>,
    base_token: &str) 
    {
        //Concstruct a new map that contains the total amount for each token
        transactions.keys().zip(transactions.values())
            .for_each(
                |(x, y)| {
                    total_holdings.insert(
                        x.to_string(), 
                        y.iter().fold(0.0, |s, x| s + x.amount));
                }
            );
        
        //Construct a new map that contains the daily holdings for each token
        for (token, tx) in transactions.keys().zip(transactions.values()) {
            daily_holdings.insert(token.clone(), calculate_daily_holdings(&tx, None, Some(Utc::now().date())));
        }
        //Construct a map containing all exchange rates for tokens
        let prices = calculate_price(&perform_price_query().expect("Failed to pull price data"));
        //Construct market value of transactions 
        let mut transactions_mv = BTreeMap::new(); 
        for (k, v) in transactions.iter() {
            let mut mv = vec![];
            for tx in v.iter() {
                if k == base_token {
                    mv.push((tx.timestamp, tx.amount));
                } else {
                    let p = lookup_price(k, base_token, tx.timestamp.date(), &prices).expect(format!("No liquidity pool has been created between {} and the base token {}", k, base_token).as_str());
                    mv.push((tx.timestamp, tx.amount * p));
                }
            }
            transactions_mv.insert(k.clone(), mv);
        }
        let mut daily_transactions_mv = BTreeMap::new();
        //Construct daily aggregate of mv of transactions
        for (k, v) in transactions_mv.iter() {
            let mut m = BTreeMap::new();
            for tx in v.iter() {
                m.entry(tx.0.date())
                    .and_modify(|mv| {*mv += tx.1})
                    .or_insert(tx.1);
            }
            daily_transactions_mv.insert(k.to_string(), m);
        }
        //Construct market value of running holdings
        for (k, v) in daily_holdings.iter() {
            let mut mv = BTreeMap::new();
            for (d, a) in v.iter() {
                if k == base_token {
                    mv.insert(*d, *a);
                }
                else {
                    let p = lookup_price(k, base_token, *d, &prices).expect(format!("No liquidity pool has been created between {} and the base token {}", k, base_token).as_str());
                    mv.insert(*d, *a * p);
                }
            }
            daily_holdings_mv.insert(k.clone(), mv);
        }
        for (k, v) in daily_holdings_mv.iter() {
            total_holdings_mv.insert(k.clone(), *v.iter().last().unwrap().1);
        }

        for token in transactions.keys() {
            let mut mv_vec = vec![];
            let mut date_vec = vec![];
            for (t, mv) in daily_transactions_mv.get(token).expect("Token should have price") {
                mv_vec.push(-*mv);
                date_vec.push(*t);
            }
            //now insert the account value as of the last date
            let last_mv = total_holdings_mv
                .get(token)
                .expect("Token should have holdings data if there's a transaction");
            let last_date = daily_holdings_mv.get(token).expect("Token should have holdings data if there's a transaction").keys().last().unwrap();
            mv_vec.push(*last_mv);
            date_vec.push(*last_date);
            let payments: Vec<xirr::Payment> = mv_vec.iter().zip(date_vec.iter()).map(|(x, y)| xirr::Payment{date: y.naive_utc(), amount: *x}).collect();
            mwr_map.insert(token.clone(), xirr::compute(&payments).unwrap());
        }

        //generate cost bases
        for (token, txs) in transactions {
            if token == base_token {
                continue;
            }
            let start_date = txs.first().expect("Need at least one transaction to generate date").timestamp.date();
            let dates = generate_dates(start_date, Utc::now().date());
            let costs = calculate_cost_basis(token, base_token, txs, &prices);
            let mut costs_iter = costs.iter().peekable();
            let mut cur_date = costs_iter.next();
            let mut next_date = costs_iter.peek();
            let mut daily_cost_basis = vec![];
            for date in dates {
                if let Some(d) = next_date {
                    if date < *d.0 {
                        daily_cost_basis.push((date, *cur_date.unwrap().1));
                    } else {
                        daily_cost_basis.push((date, *d.1));
                        cur_date = costs_iter.next();
                        next_date = costs_iter.peek();
                    }
                } else {
                    //if next date doesn't exist, we're at the last transaction, so we can just use the cost basis as of this transaction
                    daily_cost_basis.push((date, *cur_date.unwrap().1));
                }
            }
            cost_bases.insert(token.to_string(), daily_cost_basis);
        }
}

fn generate_dates(mut start_date: Date<Utc>, end_date: Date<Utc>) -> Vec<Date<Utc>> {
    let mut dates = vec![];
    while start_date <= end_date {
        dates.push(start_date);
        start_date = start_date + chrono::Duration::days(1);
    }
    dates
}

//function that calculates the daily token holding given the user's transactions, as well as the time frame 
fn calculate_daily_holdings(transactions: &Vec<Transaction>, start_window_date: Option<chrono::Date<Utc>>, end_window_date: Option<chrono::Date<Utc>>) -> BTreeMap<chrono::Date<Utc>, f64> {
    //First generate a column where we have a column of days from the first transaction to the last transaction
    let start_date = transactions.first().expect("Need at least one transaction to generate data").timestamp.date();
    let end_date;
    //for end date, if there was a supplied end window date, use that as the end date. Otherwise, use the date of the last transaction.
    if let Some(d) = end_window_date {
        end_date = d;
    } else {
        end_date = transactions.last().expect("Need at least one transaction to generate data").timestamp.date();
    }
    let dates = generate_dates(start_date, end_date);
    
    //Now generate a column where each date has the net amount transacted on that day
    let date_iter = dates.iter();
    let transaction_iter = transactions.iter();
    let mut transactions_map = BTreeMap::new();
    for d in date_iter {
        transactions_map.insert(*d, 0.0);
    }  
    for t in transaction_iter {
        if let Some(x) = transactions_map.get_mut(&t.timestamp.date()) {
            *x = *x + t.amount;
        } else {
            transactions_map.insert(t.timestamp.date(), t.amount);
        }
    }
    //Now create a column that will calculate the running total in the account
    let running_total: Vec<f64> = transactions_map.values().scan(0.0, |state, x| {*state = *state + x; Some(*state)}).collect();
    let mut running_total_map = BTreeMap::new();
    for (tx, a) in transactions_map.keys().zip(running_total.iter()) {
        running_total_map.insert(*tx, *a);
    }
    running_total_map
}

//Create a price entry
//Structure of a price entry in the overall map is:: 
// (token, (date, (exchange token, ratio)))
fn get_individual_price(
    pool: &(String, String), 
    balance: &(chrono::Date<Utc>, (f64, f64)), 
    m: &mut BTreeMap<String, BTreeMap<String, BTreeMap<chrono::Date<Utc>, f64>>>,
    token_index: usize) {
        let mut overall_token = &String::new(); 
        let mut numerator = 0.0;
        let mut denominator = 1.0;
        let mut exchange_token = &String::new();
        match token_index {
            0 => {
                overall_token = &pool.0;
                numerator = balance.1.1;
                denominator = balance.1.0;
                exchange_token = &pool.1;
            },
            1 => {
                overall_token = &pool.1;
                numerator = balance.1.0;
                denominator = balance.1.1;
                exchange_token = &pool.0;
            }, 
            _ => {}
        };
        m.entry(overall_token.to_string())
        .and_modify(|e| {
            e.entry(exchange_token.to_string())
                .and_modify(|d| {d.insert(balance.0.clone(),  numerator / denominator);})
                .or_insert({
                    let mut map = BTreeMap::new();
                    map.insert(balance.0.clone(), numerator / denominator);
                    map
                });
            }
        )
        .or_insert(
            {
                let mut t = BTreeMap::new();
                let mut d = BTreeMap::new();
                d.insert(balance.0.clone(), numerator / denominator);
                t.insert(exchange_token.to_string(), d.clone());
                t
            }
        );
    }

fn calculate_price(pools: &BTreeMap<(String, String), Vec<(chrono::Date<Utc>, (f64, f64))>>) -> BTreeMap<String, BTreeMap<String, BTreeMap<chrono::Date<Utc>, f64>>> {
    let mut currency_map: BTreeMap<String, BTreeMap<String, BTreeMap<chrono::Date<Utc>, f64>>> = BTreeMap::new();
    for (k, v) in pools {
        for balance in v.iter() {
            get_individual_price(k, balance, &mut currency_map, 0);
            get_individual_price(k, balance, &mut currency_map, 1);
        }
    }
    currency_map
}
//get price from the date or the closest date to it (in case current date doesn't have a price)
fn lookup_price(
    token: &String, 
    price_token: &str,
    date: chrono::Date<Utc>, 
    prices: &BTreeMap<String, BTreeMap<String, BTreeMap<chrono::Date<Utc>, f64>>>) -> Option<f64> {
        let price_map = prices
                    .get(token)
                    .expect("Couldn't find prices for the token. If transaction exists a price must exist.");
        let exchange_map = price_map
                    .get(price_token);
        if let Some(m) = exchange_map {
            //grab the latest date that is either the date itself, or preceding it
            if let Some(x) = m.iter().take_while(|d| *d.0 <= date).last() {
                Some(*x.1)
            } else {
                //if date we want is before any price data exists,
                //go backwards until we find the earliest date that follows the date we are looking for
                //this must return, because we have at least one date in our price map since the price map exists
                Some(*(m.iter().rev().take_while(|d| *d.0 > date).last().unwrap().1))
            }
        } else {
           None 
        }
}

fn calculate_cost_basis(
    token: &String,
    base_token: &str,
    transactions:  &Vec<Transaction>,
    prices: &BTreeMap<String, BTreeMap<String, BTreeMap<chrono::Date<Utc>, f64>>>) -> BTreeMap<Date<Utc>, f64> {
        let mut cost_bases = Vec::<(Date<Utc>, f64, f64)>::new(); 
        for tx in transactions {
            let last_cost_basis= cost_bases.iter().last();
            //if transaction is a withdrawal, we simply deduct the number of tokens, CB hasn't changed
            if tx.amount < 0.0 {
                if let Some(x) = last_cost_basis {
                    let last_n = x.1;
                    let last_cost = x.2;
                    cost_bases.push((tx.timestamp.date(), last_n + tx.amount, last_cost));
                }
            } else {
            //if tx is a deposit, we need to find the new cost basis
                if let Some(x) = last_cost_basis {
                    let n_tokens = x.1 + tx.amount;
                    let price = lookup_price(&token, &base_token, tx.timestamp.date(), prices).expect(format!("No liquidity pool has been created between {} and the base token {}", token, base_token).as_str());
                    let new_cost = (x.1*x.2 + tx.amount * price) / n_tokens;
                    cost_bases.push((tx.timestamp.date(), n_tokens, new_cost));
                } else {
                    let price = lookup_price(&token, &base_token, tx.timestamp.date(), prices).expect(format!("No liquidity pool has been created between {} and the base token {}", token, base_token).as_str());
                    cost_bases.push((tx.timestamp.date(), tx.amount, tx.amount * price));
                }
            }
        }
        //now go through list of transactions, create an entry for each date.
        //since vector is in order, the last entry for a date will have the cost basis as of the end of the day
        let mut cost_map = BTreeMap::new();
        for c in cost_bases {
            cost_map.insert(c.0, c.1 * c.2);
        }
        cost_map
}