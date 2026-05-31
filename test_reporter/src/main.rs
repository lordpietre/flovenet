use std::fs;
use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct TestReport {
    timestamp: String,
    total_scenarios: usize,
    passed_scenarios: usize,
    failed_scenarios: usize,
    total_checks: usize,
    passed_checks: usize,
    duration_ms: u64,
    scenarios: Vec<ScenarioResult>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct ScenarioResult {
    name: String,
    passed: bool,
    duration_ms: u64,
    checks: Vec<TestCheck>,
    error: Option<String>,
}

#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct TestCheck {
    name: String,
    passed: bool,
    expected: String,
    actual: String,
}

#[derive(Parser)]
#[command(name = "flovenet-report", about = "Flovenet test result analyzer")]
struct Cli {
    /// Path to one or more JSON report files
    #[arg(required = true)]
    paths: Vec<PathBuf>,

    /// Output format (terminal, markdown, json)
    #[arg(short, long, default_value = "terminal")]
    format: String,

    /// Output file (default: stdout)
    #[arg(short, long)]
    output: Option<PathBuf>,
}

fn render_terminal(report: &TestReport) -> String {
    let status = if report.failed_scenarios == 0 {
        "✅ ALL PASSED"
    } else {
        "❌ SOME FAILED"
    };
    let mut out = format!(
        "\n═══ {status} ═══\n\
         Time:     {ts}\n\
         Duration: {dur}s\n\
         Scenarios: {passed}/{total} passed\n\
         Checks:    {ck_p}/{ck_t} passed ({pct:.1}%)\n\n",
        status = status,
        ts = report.timestamp,
        dur = report.duration_ms as f64 / 1000.0,
        passed = report.passed_scenarios,
        total = report.total_scenarios,
        ck_p = report.passed_checks,
        ck_t = report.total_checks,
        pct = if report.total_checks > 0 {
            report.passed_checks as f64 / report.total_checks as f64 * 100.0
        } else {
            100.0
        },
    );

    for s in &report.scenarios {
        let icon = if s.passed { "✅" } else { "❌" };
        out.push_str(&format!(
            "{icon} {} ({:.1}s)\n",
            s.name,
            s.duration_ms as f64 / 1000.0
        ));
        for c in &s.checks {
            let ck = if c.passed { "  ✅" } else { "  ❌" };
            out.push_str(&format!("{ck} {}\n", c.name));
            if !c.passed {
                out.push_str(&format!("     expected: {}\n", c.expected));
                out.push_str(&format!("     actual:   {}\n", c.actual));
            }
        }
        if let Some(err) = &s.error {
            out.push_str(&format!("     error: {err}\n"));
        }
        out.push('\n');
    }
    out
}

fn render_markdown(report: &TestReport) -> String {
    let status = if report.failed_scenarios == 0 {
        "✅ **ALL PASSED**"
    } else {
        "❌ **SOME FAILED**"
    };
    let mut out = format!(
        "# Flovenet Test Report\n\n\
         {status}\n\n\
         | Metric | Value |\n\
         |--------|-------|\n\
         | Timestamp | {} |\n\
         | Duration | {:.1}s |\n\
         | Scenarios | {}/{} |\n\
         | Checks | {}/{} ({:.0}%) |\n\n",
        report.timestamp,
        report.duration_ms as f64 / 1000.0,
        report.passed_scenarios,
        report.total_scenarios,
        report.passed_checks,
        report.total_checks,
        if report.total_checks > 0 {
            report.passed_checks as f64 / report.total_checks as f64 * 100.0
        } else {
            100.0
        },
    );

    for s in &report.scenarios {
        let icon = if s.passed { "✅" } else { "❌" };
        out.push_str(&format!("## {} {}\n\n", icon, s.name));
        out.push_str(&format!(
            "- Duration: {:.1}s\n",
            s.duration_ms as f64 / 1000.0
        ));
        out.push_str("| Check | Status | Expected | Actual |\n");
        out.push_str("|-------|--------|----------|--------|\n");
        for c in &s.checks {
            let ck = if c.passed { "✅" } else { "❌" };
            out.push_str(&format!(
                "| {} | {ck} | `{}` | `{}` |\n",
                c.name, c.expected, c.actual
            ));
        }
        if let Some(err) = &s.error {
            out.push_str(&format!("\n**Error:** {err}\n"));
        }
        out.push('\n');
    }
    out
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut all_reports: Vec<TestReport> = Vec::new();

    for path in &cli.paths {
        let data = fs::read_to_string(path)?;
        let report: TestReport = serde_json::from_str(&data)?;
        all_reports.push(report);
    }

    // Aggregate: if multiple reports, merge
    let report = if all_reports.len() == 1 {
        all_reports.remove(0)
    } else {
        let total_dur: u64 = all_reports.iter().map(|r| r.duration_ms).sum();
        let scenarios: Vec<ScenarioResult> =
            all_reports.into_iter().flat_map(|r| r.scenarios).collect();
        let passed = scenarios.iter().filter(|s| s.passed).count();
        let total_checks: usize = scenarios.iter().map(|s| s.checks.len()).sum();
        let passed_checks: usize = scenarios
            .iter()
            .flat_map(|s| s.checks.iter())
            .filter(|c| c.passed)
            .count();

        TestReport {
            timestamp: chrono::Utc::now().to_rfc3339(),
            total_scenarios: scenarios.len(),
            passed_scenarios: passed,
            failed_scenarios: scenarios.len() - passed,
            total_checks,
            passed_checks,
            duration_ms: total_dur,
            scenarios,
        }
    };

    let output = match cli.format.as_str() {
        "markdown" | "md" => render_markdown(&report),
        "json" => serde_json::to_string_pretty(&report)?,
        _ => render_terminal(&report),
    };

    if let Some(path) = cli.output {
        fs::write(&path, &output)?;
    } else {
        println!("{output}");
    }

    if report.failed_scenarios > 0 {
        std::process::exit(1);
    }
    Ok(())
}
