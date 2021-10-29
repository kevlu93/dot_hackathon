use tui::{
    backend::Backend, 
    Frame,
    layout::{Layout, Constraint, Direction, Alignment, Rect},
    style::{Color,Style,Modifier},
    symbols,
    text::{Span, Spans},
    widgets::{
        Block, 
        Borders, 
        Paragraph, 
        Row, 
        Tabs, 
        Table, 
        List, 
        ListItem, 
        Dataset, 
        GraphType, 
        Chart, 
        Axis}
};
use crate::tui_app::App;


pub fn draw<B: Backend>(f: &mut Frame<B>, app: &mut App) {
    //create tabs first
    let tab_titles = vec!["Home", "Wallet", "Liquidity Pools"].iter().cloned().map(Spans::from).collect();
    let tabs = Tabs::new(tab_titles)
        .block(Block::default().title("Tabs").borders(Borders::ALL))
        .highlight_style(Style::default().fg(Color::Cyan))
        .select(app.selected_tab);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(
            [
                Constraint::Percentage(15),
                Constraint::Percentage(85),
            ].as_ref()
        )
        .split(f.size());
    f.render_widget(tabs, chunks[0]);
    match app.selected_tab {
        0 => draw_home_tab(f, chunks[1]),
        1 => draw_wallet_tab(f, app, chunks[1]),
        //2 => draw_lp_tab(f, app, chunks[1]),
        _ => {},
    }
}

fn draw_home_tab<B>(f: &mut Frame<B>, area: Rect) where B: Backend {
    //create home page paragraph
    let home_text = vec![
        Spans::from(Span::raw("Welcome to the app!")),
        Spans::from(Span::raw("Press \'q\' to quit, \'h\' to go to home page, \'w\' to go to wallet tab, or \'l\' to go to liquidity pool page")),
    ];
    let home_paragraph = Paragraph::new(home_text)
        .block(Block::default().borders(Borders::ALL))
        .alignment(Alignment::Center);
    f.render_widget(home_paragraph, area);
}

fn draw_wallet_tab<B>(f: &mut Frame<B>, app: &mut App, area: Rect) where B: Backend {
    let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .margin(2)
                .constraints(
                    [
                        Constraint::Percentage(20),
                        Constraint::Percentage(80),
                    ].as_ref()
                ).split(area);
    let data_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints(
            [
                Constraint::Percentage(80),
                Constraint::Percentage(20),
            ]
        ).split(chunks[1]);
    draw_wallet_list(f, app, chunks[0]);
    draw_wallet_mv_chart(f, app, data_chunks[0]);
    draw_wallet_summary_table(f, app, data_chunks[1]);
}

fn draw_wallet_list<B>(f: &mut Frame<B>, app: &mut App, area: Rect) where B: Backend {
    // Draw wallet list
    let wallet_tokens: Vec<ListItem> = app
        .wallet_list_state
        .items
        .iter()
        .map(|i| ListItem::new(vec![Spans::from(Span::raw(i))]))
        .collect();

    let wallet_list = List::new(wallet_tokens)
        .block(Block::default().borders(Borders::ALL).title("Tokens"))
        .highlight_style(Style::default().add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    app.selected_token = app.wallet_list_state.items.get(app.wallet_list_state.state.selected().unwrap()).unwrap().to_string();
    f.render_stateful_widget(wallet_list, area, &mut app.wallet_list_state.state);
}

fn draw_wallet_mv_chart<B>(f: &mut Frame<B>, app: &mut App, area: Rect) where B: Backend {
    //create wallet line chart dataset
    let mv_datapoints: Vec<(f64, f64)> = app.daily_holdings_mv
        .get(&app.selected_token)
        .unwrap()
        .values()
        .enumerate()
        .map(|(x,y)|(0.0 + x as f64, *y)).collect();
    

    let mut datasets = vec![
        Dataset::default()
            .name("Holdings Over Time")
            .graph_type(GraphType::Line)
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(Color::LightGreen))
            .data(&mv_datapoints),
    ];

    let mut cb_datapoints = Vec::<(f64, f64)>::new();
    if app.selected_token != app.base_token {
        app.cost_bases
            .get(&app.selected_token)
            .unwrap()
            .iter()
            .enumerate()
            .for_each(|(x, y)| cb_datapoints.push((0.0 + x as f64, y.1)));
        datasets.push(Dataset::default()
            .name("Cost Over Time")
            .graph_type(GraphType::Line)
            .marker(symbols::Marker::Braille)
            .style(Style::default().fg(Color::LightRed))
            .data(&cb_datapoints));
    }
    
    
    let first_datapoint = mv_datapoints.iter().map(|x| x.1).next().expect("There should be transactions data");
    let mut y_max = first_datapoint;
    let mut y_min = first_datapoint;
    if app.selected_token == app.base_token {
        for v in mv_datapoints.iter().map(|x| x.1) {
            y_min = f64::min(y_min, v);
            y_max = f64::max(y_max, v);
        }
    } else {
        for (x, y) in mv_datapoints.iter().zip(cb_datapoints.iter()) {
            let min_check = f64::min(x.1, y.1);
            let max_check = f64::max(x.1, y.1);
            y_min = f64::min(y_min, min_check);
            y_max = f64::max(y_max, max_check);
        }
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
    let dates = app.daily_holdings_mv.get(&app.selected_token).unwrap();
    let x_min = dates.keys().next().expect("We should have dates here");
    let x_max = dates.keys().last().expect("We should have a last date");
    let wallet_chart = Chart::new(datasets).block(Block::default().borders(Borders::ALL))
        .x_axis(Axis::default()
            .title(Span::styled("Date", Style::default().fg(Color::White)))
            .style(Style::default().fg(Color::White))
            .bounds([0.0, mv_datapoints.last().expect("We should have datapoints here").0])
            .labels([x_min, x_max].iter().map(|d| Span::from(format!("{}", d.format("%D")))).collect()))
        .y_axis(Axis::default()
            .title(Span::styled("Amount", Style::default().fg(Color::White)))
            .style(Style::default().fg(Color::White))
            .bounds([y_min, y_max])
            .labels([y_min, y_mid, y_max].iter().map(|s| Span::from(format!("{:.2e}", f64::round(*s)))).collect())
        );
    f.render_widget(wallet_chart, area);
}

fn draw_wallet_summary_table<B>(f: &mut Frame<B>, app: &mut App, area: Rect) where B: Backend {
    //create wallet paragraph
    let wallet_table_text = vec![
        format!("{}", app.total_holdings.get(&app.selected_token).unwrap()),
        String::from(format!("{} {}", app.total_holdings_mv.get(&app.selected_token).unwrap(), app.base_token)),
        String::from(format!("{:.2}%", app.mwr_map.get(&app.selected_token).unwrap() * 100.0)),
    ];

    let wallet_table = Table::new(vec![Row::new(wallet_table_text)])
        .block(Block::default().title(format!("{} Statistics", app.selected_token)).borders(Borders::ALL))
        .header(Row::new(vec!["# of Tokens", "Value in USD", "Money-Weighted Return"])
            .style(Style::default()
            .add_modifier(Modifier::BOLD)
            .add_modifier(Modifier::UNDERLINED)))
        .style(Style::default().fg(Color::White))
        .widths(&[Constraint::Percentage(33), Constraint::Percentage(33), Constraint::Percentage(33)]);
    f.render_widget(wallet_table, area);
}


    