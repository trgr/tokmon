use anyhow::Result;
use chrono::{Duration, Utc};
use std::collections::HashMap;

use crate::pricing;
use crate::storage::Database;

pub fn run_report(range: &str, group_by: &str) -> Result<()> {
    let db = Database::open()?;

    let since = match range {
        "today" => Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap(),
        "7d" => (Utc::now() - Duration::days(7)).naive_utc(),
        "30d" => (Utc::now() - Duration::days(30)).naive_utc(),
        "all" => chrono::DateTime::from_timestamp(0, 0).unwrap().naive_utc(),
        _ => {
            // Try parsing as Nd
            if let Some(days) = range.strip_suffix('d').and_then(|d| d.parse::<i64>().ok()) {
                (Utc::now() - Duration::days(days)).naive_utc()
            } else {
                eprintln!("Unknown range '{}', using 'today'", range);
                Utc::now().date_naive().and_hms_opt(0, 0, 0).unwrap()
            }
        }
    };

    let since_utc = chrono::DateTime::<Utc>::from_naive_utc_and_offset(since, Utc);
    let requests = db.get_requests_since(since_utc)?;

    if requests.is_empty() {
        println!("No requests found for range: {}", range);
        return Ok(());
    }

    // Aggregate
    let mut total_cost = 0.0;
    let mut total_input = 0i64;
    let mut total_output = 0i64;
    let mut total_cached = 0i64;

    struct Group {
        requests: usize,
        input_tokens: i64,
        output_tokens: i64,
        cached_tokens: i64,
        cost: f64,
        avg_latency_sum: i64,
    }

    let mut groups: HashMap<String, Group> = HashMap::new();

    for req in &requests {
        total_cost += req.cost_usd;
        total_input += req.input_tokens;
        total_output += req.output_tokens;
        total_cached += req.cached_tokens;

        let key = match group_by {
            "model" => req.model.clone(),
            "provider" => req.provider.clone(),
            "session" => req.session_id[..8].to_string(),
            _ => req.session_id[..8].to_string(),
        };

        let group = groups.entry(key).or_insert(Group {
            requests: 0,
            input_tokens: 0,
            output_tokens: 0,
            cached_tokens: 0,
            cost: 0.0,
            avg_latency_sum: 0,
        });

        group.requests += 1;
        group.input_tokens += req.input_tokens;
        group.output_tokens += req.output_tokens;
        group.cached_tokens += req.cached_tokens;
        group.cost += req.cost_usd;
        group.avg_latency_sum += req.latency_ms;
    }

    // Header
    println!();
    println!(
        "\x1b[1m  tokmon report\x1b[0m — {} ({} requests)",
        range,
        requests.len()
    );
    println!(
        "  {} input + {} output tokens | {} total",
        pricing::format_tokens(total_input),
        pricing::format_tokens(total_output),
        pricing::format_cost(total_cost),
    );
    if total_cached > 0 {
        println!("  {} cached tokens", pricing::format_tokens(total_cached));
    }
    println!();

    // Table header
    println!(
        "  \x1b[90m{:<20} {:>6} {:>10} {:>10} {:>10} {:>10}\x1b[0m",
        group_by.to_uppercase(),
        "REQS",
        "IN TOKENS",
        "OUT TOKENS",
        "COST",
        "AVG MS"
    );
    println!("  {}", "\x1b[90m─\x1b[0m".repeat(70));

    // Sort by cost descending
    let mut sorted: Vec<_> = groups.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cost.partial_cmp(&a.1.cost).unwrap());

    for (key, group) in &sorted {
        let avg_latency = if group.requests > 0 {
            group.avg_latency_sum / group.requests as i64
        } else {
            0
        };

        // Cost bar
        let bar_width = if total_cost > 0.0 {
            ((group.cost / total_cost) * 20.0) as usize
        } else {
            0
        };
        let bar: String = "█".repeat(bar_width);

        println!(
            "  {:<20} {:>6} {:>10} {:>10} {:>10} {:>7}ms  \x1b[33m{}\x1b[0m",
            truncate(key, 20),
            group.requests,
            pricing::format_tokens(group.input_tokens),
            pricing::format_tokens(group.output_tokens),
            pricing::format_cost(group.cost),
            avg_latency,
            bar,
        );
    }

    println!();

    // Budget info
    if let Ok(Some(budget)) = db.get_budget() {
        if let Ok(today_cost) = db.get_today_cost() {
            let pct = (today_cost / budget) * 100.0;
            let color = if today_cost > budget {
                "31" // red
            } else if pct > 80.0 {
                "33" // yellow
            } else {
                "32" // green
            };
            println!(
                "  budget: \x1b[{}m{}\x1b[0m / {} ({:.0}%)",
                color,
                pricing::format_cost(today_cost),
                pricing::format_cost(budget),
                pct,
            );
            println!();
        }
    }

    Ok(())
}

pub fn run_log(count: usize) -> Result<()> {
    let db = Database::open()?;
    let requests = db.get_recent_requests(count)?;

    if requests.is_empty() {
        println!("No requests logged yet.");
        return Ok(());
    }

    println!();
    println!(
        "  \x1b[90m{:<12} {:<10} {:<25} {:>10} {:>10} {:>10} {:>8}\x1b[0m",
        "TIME", "PROVIDER", "MODEL", "IN", "OUT", "COST", "LATENCY"
    );
    println!("  {}", "\x1b[90m─\x1b[0m".repeat(88));

    for req in &requests {
        let time = req.timestamp.format("%H:%M:%S");
        let stream_marker = if req.is_stream { "⇄" } else { " " };

        println!(
            "  {:<12} {:<10} {:<25} {:>10} {:>10} {:>10} {:>6}ms {}",
            time,
            req.provider,
            truncate(&req.model, 25),
            pricing::format_tokens(req.input_tokens),
            pricing::format_tokens(req.output_tokens),
            pricing::format_cost(req.cost_usd),
            req.latency_ms,
            stream_marker,
        );
    }
    println!();

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max - 1])
    }
}
