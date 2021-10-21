use ::reqwest::blocking::Client;
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};
use std::io;
use tui::Terminal;
use tui::backend::TermionBackend;
use termion::raw::IntoRawMode;
use std::error::Error;
use std::env;
use tui::widgets::{Block, Borders, Paragraph, Row, Tabs, Table, Wrap, List, ListItem, ListState, Dataset, GraphType, Chart, Axis};
use tui::layout::{Layout, Constraint, Direction, Alignment};
use tui::text::{Span, Spans};
use tui::style::{Color,Style,Modifier};
use tui::symbols;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use termion::event::Key;
use termion::input::TermRead;
use chrono;
use chrono::offset::Utc;
use std::collections::{BTreeMap, HashMap};
use financial;


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

enum Event<I> {
    Input(I),
    Tick,
}

//a stateful list shows a list, highlighting the current value
pub struct StatefulList<T> {
    pub state: ListState,
    pub items: Vec<T>,
}

impl<T> StatefulList<T> {
    pub fn new() -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items: Vec::new(),
        }
    }

    pub fn with_items(items: Vec<T>) -> StatefulList<T> {
        StatefulList {
            state: ListState::default(),
            items,
        }
    }

    pub fn next(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.items.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn previous(&mut self) {
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.items.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    pub fn unselect(&mut self) {
        self.state.select(None);
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

    println!("{:?}", currency_map);
        
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


//function that calculates the daily token holding given the user's transactions, as well as the time frame 
fn calculate_daily_holdings(transactions: &Vec<Transaction>, start_window_date: Option<chrono::Date<Utc>>, end_window_date: Option<chrono::Date<Utc>>) -> BTreeMap<chrono::Date<Utc>, f64> {
    //First generate a column where we have a column of days from the first transaction to the last transaction
    let mut start_date = transactions.first().expect("Need at least one transaction to generate data").timestamp.date();
    let end_date;
    //for end date, if there was a supplied end window date, use that as the end date. Otherwise, use the date of the last transaction.
    if let Some(d) = end_window_date {
        end_date = d;
    } else {
        end_date = transactions.last().expect("Need at least one transaction to generate data").timestamp.date();
    }
    let mut dates = vec![];
    
    while start_date <= end_date {
        dates.push(start_date);
        start_date = start_date + chrono::Duration::days(1);
    }
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

fn get_individual_price(
    pool: &(String, String), 
    balance: &(chrono::Date<Utc>, (f64, f64)), 
    m: &mut BTreeMap<String, BTreeMap<chrono::Date<Utc>, BTreeMap<String, f64>>>,
    token_index: usize) {
        let mut overall_token = String::new(); 
        let mut numerator = 0.0;
        let mut denominator = 1.0;
        let mut exchange_token = String::new();
        match token_index {
            0 => {
                overall_token = pool.0.clone();
                numerator = balance.1.1;
                denominator = balance.1.0;
                exchange_token = pool.1.clone();
            },
            1 => {
                overall_token = pool.1.clone();
                numerator = balance.1.0;
                denominator = balance.1.1;
                exchange_token = pool.0.clone();
            }, 
            _ => {}
        };
        m.entry(overall_token)
        .and_modify(|e| {
            e.entry(balance.0)
                .and_modify(|d| {d.insert(exchange_token.clone(),  numerator / denominator);})
                .or_insert({
                    let mut map = BTreeMap::new();
                    map.insert(exchange_token.clone(), numerator / denominator);
                    map
                });
            }
        )
        .or_insert(
            {
                let mut t = BTreeMap::new();
                let mut d = BTreeMap::new();
                d.insert(exchange_token.clone(), numerator / denominator);
                t.insert(balance.0, d.clone());
                t
            }
        );
    }

fn calculate_price(pools: &BTreeMap<(String, String), Vec<(chrono::Date<Utc>, (f64, f64))>>) -> BTreeMap<String, BTreeMap<chrono::Date<Utc>, BTreeMap<String, f64>>> {
    let mut currency_map: BTreeMap<String, BTreeMap<chrono::Date<Utc>, BTreeMap<String, f64>>> = BTreeMap::new();
    for (k, v) in pools {
        for balance in v.iter() {
            get_individual_price(k, balance, &mut currency_map, 0);
            get_individual_price(k, balance, &mut currency_map, 1);
        }
    }
    currency_map
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let accountid = &args[1];
    
    let base_token = "AUSD";

    let transactions = perform_query(accountid).expect("Query failed for account id");
    
    let mut total_holdings = BTreeMap::new();
    //Concstruct a new map that contains the total amount for each token
    transactions.keys().zip(transactions.values()).for_each(|(x, y)| {total_holdings.insert(x, y.iter().fold(0.0, |s, x| s + x.amount));});
    
    let mut daily_holdings = BTreeMap::new();
    //Construct a new map that contains the daily holdings for each token
    for (token, tx) in transactions.keys().zip(transactions.values()) {
        daily_holdings.insert(token.clone(), calculate_daily_holdings(&tx, None, Some(Utc::now().date())));
    }
    //Construct a map containing all exchange rates for tokens
    let prices = calculate_price(&perform_price_query().expect("Failed to pull price data"));
    println!("{:?}", prices.get("ACA"));
    //Construct market value of transactions 
    let mut transactions_mv = BTreeMap::new(); 
    for (k, v) in transactions.iter() {
        let mut mv = vec![];
        for tx in v.iter() {
            let p = prices
                    .get(k)
                    .expect("Couldn't find prices for the token. If transaction exists a price must exist.")
                    .get(&tx.timestamp.date()) 
                    .expect(format!("Couldn't find price for {} on {}. If a transaction occurred there should have been a price.", k, tx.timestamp.date()).as_str())
                    .get(base_token)
                    .expect(format!("No liquidity pool has been created between {} and the base token {}", k, base_token).as_str());
            mv.push((tx.timestamp, tx.amount * *p));
        }
        transactions_mv.insert(k.clone(), mv);
    }
    //Construct market value of running holdings
    let mut daily_holdings_mv = BTreeMap::new();
    let mut total_holdings_mv = HashMap::new();
    for (k, v) in daily_holdings.iter() {
        let mut mv = BTreeMap::new();
        let mut total = 0.0;
        for (d, a) in v.iter() {
            let p = prices
                    .get(k)
                    .expect("Couldn't find prices for the token. If transaction exists a price must exist.")
                    .get(d) 
                    .expect("Couldn't find price on given date. If a transaction occurred there should have been a price.")
                    .get(base_token)
                    .expect(format!("No liquidity pool has been created between {} and the base token {}", k, base_token).as_str());
            mv.insert(*d, *a * *p);
            total += *a * *p;
        }
        daily_holdings_mv.insert(k.clone(), mv);
        total_holdings_mv.insert(k.clone(), total);
    }
    //Calculate all-time money weighted return
    let mut mwr_map = BTreeMap::new();
    for token in transactions.keys() {
        let mut mv_vec = vec![];
        let mut timestamp_vec = vec![];
        for (t, mv) in transactions_mv.get(token).expect("Token should have price") {
            mv_vec.push(-*mv);
            timestamp_vec.push(*t);
        }
        //now insert the account value as of the last date
        let last_mv = total_holdings_mv
            .get(token)
            .expect("Token should have holdings data if there's a transaction");
        let last_date = daily_holdings_mv.get(token).expect("Token should have holdings data if there's a transaction").keys().last().unwrap();
        mv_vec.push(*last_mv);
        timestamp_vec.push((*last_date + chrono::Duration::days(1)).and_hms(0,0,0));
        let mwr = financial::xirr(&mv_vec[..], &timestamp_vec[..], None).unwrap();
        mwr_map.insert(token.clone(), mwr);
    }
    println!("{:?}", mwr_map);
    

    let (tx, rx) = mpsc::channel();
    let tick_rate = Duration::from_millis(1000);
    let input_handle = {
        let tx = tx.clone();
        thread::spawn(move || {
            let stdin = io::stdin();
            for evt in stdin.keys() {
                if let Ok(key) = evt {
                    if let Err(err) = tx.send(Event::Input(key)) {
                        eprintln!("{}", err);
                        return;
                    }
                }
            }
        })
    };

    let tick_handle = {
        thread::spawn(move || loop {
            if let Err(err) = tx.send(Event::Tick) {
                eprintln!("{}", err);
                break;
            }
            thread::sleep(tick_rate);
        })
    };

    let stdout = io::stdout().into_raw_mode().unwrap();
    let backend = TermionBackend::new(stdout);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.clear().unwrap();
    let mut selected_tab = 0;
    //create wallet list
    let wallet_tokens = total_holdings.keys().collect();
    let mut wallet_list_state = StatefulList::with_items(wallet_tokens);
    wallet_list_state.state.select(Some(0));
    //create liquidity pool list
    let liquidity_pools = vec!["Token 1 <=> Token 2", "Token 1 <=> Token 3", "Token 2 <=> Token 3"];
    let mut liquidity_pool_list_state = StatefulList::with_items(liquidity_pools);
    liquidity_pool_list_state.state.select(Some(0));
    loop {
        terminal.draw(|f| {
            //create tabs first
            let tab_titles = vec!["Home", "Wallet", "Liquidity Pools"].iter().cloned().map(Spans::from).collect();
            let tabs = Tabs::new(tab_titles)
                .block(Block::default().title("Tabs").borders(Borders::ALL))
                .highlight_style(Style::default().fg(Color::Cyan))
                .select(selected_tab);

            let overall_chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints(
                    [
                        Constraint::Percentage(15),
                        Constraint::Percentage(85),
                    ].as_ref()
                )
                .split(f.size());
            f.render_widget(tabs, overall_chunks[0]);

            //create home page paragraph
            let home_text = vec![
                Spans::from(Span::raw("Welcome to the app!")),
                Spans::from(Span::raw("Press \'q\' to quit, \'h\' to go to home page, \'w\' to go to wallet tab, or \'l\' to go to liquidity pool page")),
            ];
            let home_paragraph = Paragraph::new(home_text)
                .block(Block::default().borders(Borders::ALL))
                .alignment(Alignment::Center);

            //create wallet chunks
            let wallet_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(2)
                .constraints(
                    [
                        Constraint::Percentage(20),
                        Constraint::Percentage(80),
                    ].as_ref()
                ).split(overall_chunks[1]);
            let data_chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints(
                    [
                        Constraint::Percentage(80),
                        Constraint::Percentage(20),
                    ]
                ).split(wallet_chunks[1]);
            
            // Draw wallet list
            let wallet_tokens: Vec<ListItem> = wallet_list_state
                .items
                .iter()
                .map(|i| ListItem::new(vec![Spans::from(Span::raw(**i))]))
                .collect();
            let wallet_list = List::new(wallet_tokens)
                .block(Block::default().borders(Borders::ALL).title("Tokens"))
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");

            let selected_token = wallet_list_state.items.get(wallet_list_state.state.selected().unwrap()).unwrap();
            //create wallet paragraph
            let wallet_table_text = vec![
                format!("{}", total_holdings.get(**selected_token).unwrap()),
                String::from(format!("{} {}", total_holdings_mv.get(**selected_token).unwrap(), base_token)),
                String::from(format!("{:.2}%", mwr_map.get(**selected_token).unwrap() * 100.0)),
            ];
            
            let wallet_table = Table::new(vec![Row::new(wallet_table_text)])
                .block(Block::default().title(format!("{} Statistics", selected_token)).borders(Borders::ALL))
                .header(Row::new(vec!["# of Tokens", "Value in USD", "Money-Weighted Return"])
                    .style(Style::default()
                    .add_modifier(Modifier::BOLD)
                    .add_modifier(Modifier::UNDERLINED)))
                .style(Style::default().fg(Color::White))
                .widths(&[Constraint::Percentage(33), Constraint::Percentage(33), Constraint::Percentage(33)]);

            //create wallet line chart dataset
            let datapoints: Vec<(f64, f64)> = daily_holdings.get(**selected_token).unwrap().values().enumerate().map(|(x,y)|(0.0 + x as f64, *y)).collect();
            let datasets = vec![
                Dataset::default()
                    .name("Holdings Over Time")
                    .graph_type(GraphType::Line)
                    .marker(symbols::Marker::Braille)
                    .style(Style::default().fg(Color::White))
                    .data(&datapoints),
            ];
            let first_datapoint = datapoints.iter().map(|x| x.1).next().expect("There should be transactions data");
            let mut y_max = first_datapoint;
            let mut y_min = first_datapoint;
            for v in datapoints.iter().map(|x| x.1) {
                y_min = f64::min(y_min, v);
                y_max = f64::max(y_max, v);
            }
            if y_min < 0.0 {
                y_min = 1.1 * y_min;
            } else {
                y_min = 0.9 * y_min;
            }
            if y_max < 0.0 {
                y_max = 0.9 * y_max;
            } else {
                y_max = 1.1 * y_max;
            }
            let y_mid;
            if y_max < 0.0 {
               y_mid = (y_min - y_max) / 2.0; 
            } else {
                y_mid = (y_max - y_min) / 2.0;
            }
            let dates = daily_holdings.get(**selected_token).unwrap();
            let x_min = dates.keys().next().expect("We should have dates here");
            let x_max = dates.keys().last().expect("We should have a last date");
            let wallet_chart = Chart::new(datasets).block(Block::default().borders(Borders::ALL))
                .x_axis(Axis::default()
                    .title(Span::styled("Date", Style::default().fg(Color::White)))
                    .style(Style::default().fg(Color::White))
                    .bounds([0.0, datapoints.last().expect("We should have datapoints here").0])
                    .labels([x_min, x_max].iter().map(|d| Span::from(format!("{}", d.format("%D")))).collect()))
                .y_axis(Axis::default()
                    .title(Span::styled("Amount", Style::default().fg(Color::White)))
                    .style(Style::default().fg(Color::White))
                    .bounds([y_min, y_max])
                    .labels([y_min, y_mid, y_max].iter().map(|s| Span::from(format!("{:.2e}", f64::round(*s)))).collect())
                );

            // Draw lp list
            let liquidity_pools: Vec<ListItem> = liquidity_pool_list_state
                .items
                .iter()
                .map(|i| ListItem::new(vec![Spans::from(Span::raw(*i))]))
                .collect();
            let lp_list = List::new(liquidity_pools)
                .block(Block::default().borders(Borders::ALL).title("Liquidity Pools"))
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");

            let selected_pool = liquidity_pool_list_state.items.get(liquidity_pool_list_state.state.selected().unwrap()).unwrap();
            //create wallet paragraph
            let lp_text = vec![
                Spans::from(Span::raw(format!("100 {} Pool Share Tokens", selected_pool))),
                Spans::from(Span::raw("500 USD")),
                Spans::from(Span::raw("+12%")),
            ];
            let lp_paragraph = Paragraph::new(lp_text)
                .block(Block::default().title(format!("{} Liquidity Pool Holdings", selected_pool)).borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Center)
                .wrap(Wrap {trim: true});
            //render tab based on key selection
            match selected_tab {
                0 => f.render_widget(home_paragraph, overall_chunks[1]),
                1 => {
                        f.render_stateful_widget(wallet_list, wallet_chunks[0], &mut wallet_list_state.state);
                        f.render_widget(wallet_chart, data_chunks[0]);
                        f.render_widget(wallet_table, data_chunks[1]);
                    },
                2 => {
                    f.render_stateful_widget(lp_list, wallet_chunks[0], &mut liquidity_pool_list_state.state);
                    f.render_widget(lp_paragraph, wallet_chunks[1]);
                }
                _ => {}
            }
        }).unwrap();

        match rx.recv().unwrap() {
            Event::Input(key) => match key {
                Key::Char('q') => {
                    terminal.show_cursor().unwrap();
                    terminal.clear().unwrap();
                    break;
                },
                Key::Char('h') => {
                    selected_tab = 0;
                }
                Key::Char('w') => {
                    selected_tab = 1;
                },
                Key::Char('l') => {
                    selected_tab = 2;
                },
                Key::Up => {
                    match selected_tab {
                        1 => wallet_list_state.previous(),
                        2 => liquidity_pool_list_state.previous(),
                        _ => {}
                    }
                },
                Key::Down => {
                    match selected_tab {
                        1 => wallet_list_state.next(),
                        2 => liquidity_pool_list_state.next(),
                        _ => {}
                    }
                }
                _ => {}
            },
            Event::Tick => {}
        }
    }
    
}
