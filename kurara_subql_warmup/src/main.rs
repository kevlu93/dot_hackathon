use ::reqwest::blocking::Client;
use graphql_client::{reqwest::post_graphql_blocking as post_graphql, GraphQLQuery};
use std::io;
use tui::Terminal;
use tui::backend::TermionBackend;
use termion::raw::IntoRawMode;
use std::error::Error;
//use std::env;
use tui::widgets::{Block, Borders, Paragraph, Tabs, Wrap, List, ListItem, ListState};
use tui::layout::{Layout, Constraint, Direction, Alignment};
use tui::text::{Span, Spans};
use tui::style::{Color,Style,Modifier};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use termion::event::Key;
use termion::input::TermRead;


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

fn perform_query(account_id: &String) -> Result<(), Box<dyn Error>> {
    let variables = transfers_query::Variables{
        accountid: account_id.to_string(),
    };

    let client = Client::new();

    let response_body = post_graphql::<TransfersQuery, _>(&client, "https://api.subquery.network/sq/AcalaNetwork/karura", variables).unwrap();
    //println!("{:#?}", response_body);

    let response_data = &response_body.data.expect("missing response data");
    let mut transfer_in_amounts: Vec<i64> = Vec::new();
    let mut transfer_out_amounts: Vec<i64> = Vec::new();
    let mut transfer_in_count = 0;
    let mut transfer_out_count = 0;

    //We are only looking at one account, so accounts will only have one account in its nodes vector
    if let Some(n) = &response_data.accounts.as_ref().expect("No transfers for account").nodes.iter().flat_map(|n| n.iter()).next() {
        if n.transfer_in.total_count > 0 {
            transfer_in_amounts = n
            .transfer_in.nodes.iter()
            .flat_map(|n| n.iter())
            //unwrap of amount is ok because we know we have transfers in, so it will return Some
            .map(|t| t.amount.as_ref().unwrap().parse::<i64>().expect("Not parseable as an integer!")).collect();  

            transfer_in_count = n.transfer_in.total_count;
        }

        if n.transfer_out.total_count > 0 {
            transfer_out_amounts = n
            .transfer_out.nodes.iter()
            .flat_map(|n| n.iter())
            .map(|t| t.amount.as_ref().unwrap().parse::<i64>().expect("Not parseable as an integer!")).collect();  

            transfer_out_count = n.transfer_out.total_count;
        }
    }
    
    let transfer_in_sum: i64 = transfer_in_amounts.iter().sum();
    let transfer_out_sum: i64 = transfer_out_amounts.iter().sum();

    

    transfer_in_amounts.sort();
    transfer_out_amounts.sort();

    let mut transfer_in_median: i64 = 0;  
    let mut transfer_out_median: i64 = 0; 

    let mut transfer_in_mean: i64 = 0;
    if transfer_in_count > 0 {
        transfer_in_mean = transfer_in_sum / transfer_in_count;
        transfer_in_median = median(&mut transfer_in_amounts);
    } 

    let mut transfer_out_mean: i64 = 0;
    if transfer_out_count > 0 {
        transfer_out_mean = transfer_out_sum / transfer_out_count;
        transfer_out_median = median(&mut transfer_out_amounts);
    } 

    let transfer_net_sum: i64 = transfer_in_sum - transfer_out_sum;

    //println!("Transfer Summary Statistics For AccountId {}", account_id);
    //println!("--------------------------------------------");
    //println!("Transfers Received| Total:{} KAR, Median:{} KAR, Mean:{} KAR", transfer_in_sum, transfer_in_median, transfer_in_mean);
    //println!("Transfers Sent| Total:{} KAR, Median:{} KAR, Mean:{} KAR", transfer_out_sum, transfer_out_median, transfer_out_mean);
    //println!("Net Transfer Amount: {} KAR", transfer_net_sum);
    Ok(())
}


fn main() {
    //let args: Vec<String> = env::args().collect();
    //let accountid = &args[1];
    //perform_query(accountid);
    
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
    let wallet_tokens = vec!["Token 1", "Token 2", "Token 3"];
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

            
            // Draw wallet list
            let wallet_tokens: Vec<ListItem> = wallet_list_state
                .items
                .iter()
                .map(|i| ListItem::new(vec![Spans::from(Span::raw(*i))]))
                .collect();
            let wallet_list = List::new(wallet_tokens)
                .block(Block::default().borders(Borders::ALL).title("Tokens"))
                .highlight_style(Style::default().add_modifier(Modifier::BOLD))
                .highlight_symbol("> ");

            let selected_token = wallet_list_state.items.get(wallet_list_state.state.selected().unwrap()).unwrap();
            //create wallet paragraph
            let wallet_text = vec![
                Spans::from(Span::raw(format!("100 {}", selected_token))),
                Spans::from(Span::raw("500 USD")),
                Spans::from(Span::raw("+12%")),
            ];
            let wallet_paragraph = Paragraph::new(wallet_text)
                .block(Block::default().title(format!("Token {} Holdings", selected_token)).borders(Borders::ALL))
                .style(Style::default().fg(Color::White))
                .alignment(Alignment::Center)
                .wrap(Wrap {trim: true});

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
                        f.render_widget(wallet_paragraph, wallet_chunks[1]);
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
