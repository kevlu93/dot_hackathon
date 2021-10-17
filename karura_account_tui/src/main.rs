use ::reqwest::blocking::Client;
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};
use std::io;
use tui::Terminal;
use tui::backend::TermionBackend;
use termion::raw::IntoRawMode;
use std::error::Error;
use std::env;
use tui::widgets::{Block, Borders, Paragraph, Tabs, Wrap, List, ListItem, ListState, Dataset, GraphType, Chart, Axis};
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
use chrono::offset::utc::UTC;
use std::collections::{BTreeMap, HashMap};

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "src/schema.json",
    query_path = "src/transfers_query.graphql",
    response_derives = "Debug",
)]

pub struct TransfersQuery;

struct TransferSummary {
    id: String,
    transfer_type: TransferType,
    total: i64,
    mean: i64,
    median: i64,
}

#[derive(Debug)]
struct Transaction {
    amount: f64,
    timestamp: chrono::DateTime<UTC>, 
}

enum TransferType {
    In,
    Out,
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

fn median(data: &mut Vec<i64>) -> i64 {
    data.sort();
    let mid = data.len() / 2;
    if data.len() % 2 == 0 {
        (data[mid - 1] + data[mid]) / 2
    } else {
        data[mid]
    }
}

fn perform_query(account_id: &String) -> Result<HashMap<String, Vec<Transaction>>, Box<dyn Error>> {
    let variables = transfers_query::Variables{
        accountid: account_id.to_string(),
    };

    let client = Client::new();

    let response_body = post_graphql::<TransfersQuery, _>(&client, "https://api.subquery.network/sq/kevlu93/karura-currency-transfer-tracker", variables).unwrap();
    //println!("{:#?}", response_body);
    
    
    let response_data = &response_body.data.expect("missing response data");

    let mut currency_map = HashMap::new();

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
                            UTC),
                    }
                )
            }
            currency_map.insert(c.currency.clone(), transactions);
        }
    } 

    Ok(currency_map)
}

//function that calculates the daily token holding given the user's transactions, as well as the time frame 
fn calculate_daily_holdings(transactions: &Vec<Transaction>, start_window_date: Option<chrono::Date<UTC>>, end_window_date: Option<chrono::Date<UTC>>) -> BTreeMap<chrono::Date<UTC>, f64> {
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


fn main() {
    let args: Vec<String> = env::args().collect();
    let accountid = &args[1];
    let holdings = perform_query(accountid).expect("Query failed for account id");
    
    let mut total_holdings = HashMap::new();
    //Concstruct a new map that contains the total amount for each token
    holdings.keys().zip(holdings.values()).for_each(|(x, y)| {total_holdings.insert(x, y.iter().fold(0.0, |s, x| s + x.amount));});
    
    let mut daily_holdings = HashMap::new();
    //Construct a new map that contains the daily holdings for each token
    for (token, tx) in holdings.keys().zip(holdings.values()) {
        daily_holdings.insert(token, calculate_daily_holdings(&tx, None, Some(UTC::now().date())));
    }

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
                .constraints(
                    [
                        Constraint::Percentage(20),
                        Constraint::Percentage(80),
                    ].as_ref()
                ).split(overall_chunks[1]);
            let data_chunks = Layout::default()
                .direction(Direction::Vertical)
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
            let wallet_text = vec![
                Spans::from(Span::raw(format!("Amount of Token: {} {}", total_holdings.get(**selected_token).unwrap(), selected_token))),
                Spans::from(Span::raw("500 USD")),
                Spans::from(Span::raw("+12%")),
            ];
            let wallet_paragraph = Paragraph::new(wallet_text)
                .block(Block::default().title(format!("{} Holdings", selected_token)).borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Center)
                .wrap(Wrap {trim: true});
            //create wallet line chart dataset
            let datapoints: Vec<(f64, f64)> = daily_holdings.get(**selected_token).unwrap().values().enumerate().map(|(x,y)|(0.0 + x as f64, *y)).collect();
            let datasets = vec![
                Dataset::default()
                    .name("Holdings")
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
            y_min = 1.1 * y_min;
            y_max = 1.1 * y_max;
            let wallet_chart = Chart::new(datasets).block(Block::default().title("Chart"))
                .x_axis(Axis::default()
                    .title(Span::styled("Date", Style::default().fg(Color::Red)))
                    .style(Style::default().fg(Color::White))
                    .bounds([0.0, datapoints.last().expect("We should have datapoints here").0]))
                .y_axis(Axis::default()
                    .title(Span::styled("Y Axis", Style::default().fg(Color::Red)))
                    .style(Style::default().fg(Color::White))
                    .bounds([y_min, y_max]));

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
                        f.render_widget(wallet_paragraph, data_chunks[1]);
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
