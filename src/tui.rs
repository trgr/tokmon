use anyhow::Result;
use chrono::{Duration, Utc};
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Cell, Padding, Paragraph, Row, Table},
};
use std::io::stdout;

use crate::pricing;
use crate::storage::Database;

pub async fn run_tui() -> Result<()> {
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let result = run_app(&mut terminal).await;

    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    result
}

async fn run_app(terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> Result<()> {
    loop {
        let db = Database::open()?;

        // Gather data
        let today_start = Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap();
        let today_utc = chrono::DateTime::<Utc>::from_naive_utc_and_offset(today_start, Utc);
        let week_ago = Utc::now() - Duration::days(7);

        let today_requests = db.get_requests_since(today_utc)?;
        let week_requests = db.get_requests_since(week_ago)?;
        let recent = db.get_recent_requests(30)?;
        let _sessions = db.get_sessions_since(week_ago)?;
        let budget = db.get_budget()?;
        let today_cost = db.get_today_cost()?;

        // Aggregate today
        let today_total_cost: f64 = today_requests.iter().map(|r| r.cost_usd).sum();
        let today_input: i64 = today_requests.iter().map(|r| r.input_tokens).sum();
        let today_output: i64 = today_requests.iter().map(|r| r.output_tokens).sum();
        let week_total_cost: f64 = week_requests.iter().map(|r| r.cost_usd).sum();

        // Model breakdown
        let mut model_costs: std::collections::HashMap<String, (usize, i64, i64, f64)> =
            std::collections::HashMap::new();
        for req in &today_requests {
            let entry = model_costs.entry(req.model.clone()).or_default();
            entry.0 += 1;
            entry.1 += req.input_tokens;
            entry.2 += req.output_tokens;
            entry.3 += req.cost_usd;
        }

        terminal.draw(|frame| {
            let area = frame.area();

            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(5),  // Header / summary
                    Constraint::Min(10),    // Main content
                    Constraint::Length(1),  // Footer
                ])
                .split(area);

            // ─── Header ───
            let budget_str = if let Some(b) = budget {
                let pct = (today_cost / b) * 100.0;
                let color = if today_cost > b {
                    Style::default().fg(Color::Red).bold()
                } else if pct > 80.0 {
                    Style::default().fg(Color::Yellow).bold()
                } else {
                    Style::default().fg(Color::Green).bold()
                };
                Line::from(vec![
                    Span::raw("  budget: "),
                    Span::styled(
                        format!("{} / {} ({:.0}%)", pricing::format_cost(today_cost), pricing::format_cost(b), pct),
                        color,
                    ),
                ])
            } else {
                Line::from("  budget: not set (use `tokmon budget <amount>`)").style(Style::default().fg(Color::DarkGray))
            };

            let header = Paragraph::new(vec![
                Line::from(vec![
                    Span::styled(" tokmon ", Style::default().fg(Color::Cyan).bold()),
                    Span::raw("— LLM cost tracker"),
                ]),
                Line::from(""),
                Line::from(vec![
                    Span::raw("  today: "),
                    Span::styled(
                        format!("{}", pricing::format_cost(today_total_cost)),
                        Style::default().fg(Color::Yellow).bold(),
                    ),
                    Span::raw(format!(
                        "  ({} reqs, {} in / {} out)",
                        today_requests.len(),
                        pricing::format_tokens(today_input),
                        pricing::format_tokens(today_output),
                    )),
                    Span::raw("    7d: "),
                    Span::styled(
                        pricing::format_cost(week_total_cost),
                        Style::default().fg(Color::Yellow),
                    ),
                ]),
                budget_str,
            ])
            .block(
                Block::default()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::default().fg(Color::DarkGray)),
            );
            frame.render_widget(header, outer[0]);

            // ─── Main content: split into model breakdown + recent requests ───
            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(35), Constraint::Percentage(65)])
                .split(outer[1]);

            // Model breakdown table
            let mut sorted_models: Vec<_> = model_costs.into_iter().collect();
            sorted_models.sort_by(|a, b| b.1 .3.partial_cmp(&a.1 .3).unwrap());

            let model_rows: Vec<Row> = sorted_models
                .iter()
                .map(|(model, (reqs, input, output, cost))| {
                    Row::new(vec![
                        Cell::from(truncate_str(model, 22)),
                        Cell::from(format!("{}", reqs)).style(Style::default().fg(Color::White)),
                        Cell::from(pricing::format_tokens(*input)),
                        Cell::from(pricing::format_tokens(*output)),
                        Cell::from(pricing::format_cost(*cost))
                            .style(Style::default().fg(Color::Yellow)),
                    ])
                })
                .collect();

            let model_table = Table::new(
                model_rows,
                [
                    Constraint::Min(12),
                    Constraint::Length(5),
                    Constraint::Length(8),
                    Constraint::Length(8),
                    Constraint::Length(9),
                ],
            )
            .header(
                Row::new(vec!["MODEL", "REQS", "IN", "OUT", "COST"])
                    .style(Style::default().fg(Color::DarkGray))
                    .bottom_margin(1),
            )
            .block(
                Block::default()
                    .title(" Models (today) ")
                    .title_style(Style::default().fg(Color::Cyan))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .padding(Padding::horizontal(1)),
            );
            frame.render_widget(model_table, main[0]);

            // Recent requests table
            let req_rows: Vec<Row> = recent
                .iter()
                .map(|req| {
                    let time = req.timestamp.format("%H:%M:%S").to_string();
                    let stream_icon = if req.is_stream { "⇄" } else { " " };
                    Row::new(vec![
                        Cell::from(time),
                        Cell::from(req.provider.as_str()),
                        Cell::from(truncate_str(&req.model, 20)),
                        Cell::from(pricing::format_tokens(req.input_tokens)),
                        Cell::from(pricing::format_tokens(req.output_tokens)),
                        Cell::from(pricing::format_cost(req.cost_usd))
                            .style(Style::default().fg(Color::Yellow)),
                        Cell::from(format!("{}ms", req.latency_ms)),
                        Cell::from(stream_icon),
                    ])
                })
                .collect();

            let req_table = Table::new(
                req_rows,
                [
                    Constraint::Length(9),
                    Constraint::Length(10),
                    Constraint::Min(12),
                    Constraint::Length(8),
                    Constraint::Length(8),
                    Constraint::Length(9),
                    Constraint::Length(8),
                    Constraint::Length(2),
                ],
            )
            .header(
                Row::new(vec!["TIME", "PROVIDER", "MODEL", "IN", "OUT", "COST", "LATENCY", ""])
                    .style(Style::default().fg(Color::DarkGray))
                    .bottom_margin(1),
            )
            .block(
                Block::default()
                    .title(" Recent Requests ")
                    .title_style(Style::default().fg(Color::Cyan))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray))
                    .padding(Padding::horizontal(1)),
            );
            frame.render_widget(req_table, main[1]);

            // Footer
            let footer = Paragraph::new(
                Line::from(vec![
                    Span::styled(" q", Style::default().fg(Color::Cyan).bold()),
                    Span::raw(" quit  "),
                    Span::styled("r", Style::default().fg(Color::Cyan).bold()),
                    Span::raw(" refresh"),
                ])
            );
            frame.render_widget(footer, outer[2]);
        })?;

        // Poll for input with 2-second auto-refresh
        if event::poll(std::time::Duration::from_secs(2))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break,
                        KeyCode::Char('r') => continue, // force refresh
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(())
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
