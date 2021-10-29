mod utils;
mod tui_app;
mod ui;
use tui_app::App;
use std::io;
use tui::{
    backend::TermionBackend,
    Terminal,
};
use termion::{
    raw::IntoRawMode,
    event::Key,
    input::TermRead,
};
use std::env;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

enum Event<I> {
    Input(I),
    Tick,
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let accountid = &args[1];

    let mut app = App::new(accountid.to_string());
    
    //app.daily_holdings_mv.get("DOT").unwrap().iter().zip(app.cost_bases.get("DOT").unwrap().iter()).for_each(|(x,y)| println!("({},{})  ({},{})", x.0, x.1, y.0, y.1));

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
    
    loop {
        terminal.draw(|f| {
            ui::draw(f, &mut app);
        }).unwrap();

        match rx.recv().unwrap() {
            Event::Input(key) => match key {
                Key::Char('q') => {
                    terminal.show_cursor().unwrap();
                    terminal.clear().unwrap();
                    break;
                },
                Key::Char('h') => {
                    app.selected_tab = 0;
                }
                Key::Char('w') => {
                    app.selected_tab = 1;
                },
                Key::Char('l') => {
                    app.selected_tab = 2;
                },
                Key::Up => {
                    match app.selected_tab {
                        1 => app.wallet_list_state.previous(),
                        //2 => liquidity_pool_list_state.previous(),
                        _ => {}
                    }
                },
                Key::Down => {
                    match app.selected_tab {
                        1 => app.wallet_list_state.next(),
                        //2 => liquidity_pool_list_state.next(),
                        _ => {}
                    }
                }
                _ => {}
            },
            Event::Tick => {}
        }
    }
    
}
