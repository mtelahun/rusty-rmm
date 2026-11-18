//! Tables for people, the same rows as JSON for scripts (`--json`).

use serde_json::{Map, Value};

pub struct Table {
    pub headers: Vec<&'static str>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&'static str]) -> Self {
        Self {
            headers: headers.to_vec(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn render(&self, json: bool) -> String {
        if json {
            let rows: Vec<Value> = self.rows.iter().map(|r| object(&self.headers, r)).collect();
            return serde_json::to_string_pretty(&rows).expect("strings serialize");
        }
        let widths: Vec<usize> = (0..self.headers.len())
            .map(|i| {
                self.rows
                    .iter()
                    .map(|r| r[i].chars().count())
                    .chain([self.headers[i].len()])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let line = |cells: Vec<&str>| {
            let padded: Vec<String> = cells
                .iter()
                .zip(&widths)
                .map(|(c, w)| format!("{c:<w$}"))
                .collect();
            padded.join("  ").trim_end().to_owned()
        };
        let mut out = vec![line(self.headers.clone())];
        out.extend(
            self.rows
                .iter()
                .map(|r| line(r.iter().map(String::as_str).collect())),
        );
        out.join("\n")
    }
}

/// One record as `field: value` lines, or a JSON object.
pub fn details(fields: &[(&'static str, String)], json: bool) -> String {
    if json {
        let (keys, values): (Vec<&str>, Vec<String>) = fields.iter().cloned().unzip();
        return serde_json::to_string_pretty(&object(&keys, &values)).expect("strings serialize");
    }
    let width = fields.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    fields
        .iter()
        .map(|(k, v)| format!("{k:<width$}  {v}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn object(keys: &[&str], values: &[String]) -> Value {
    let map: Map<String, Value> = keys
        .iter()
        .zip(values)
        .map(|(k, v)| (k.replace(' ', "_"), Value::String(v.clone())))
        .collect();
    Value::Object(map)
}

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn time(ts: Option<&prost_types::Timestamp>) -> String {
    ts.and_then(|t| chrono::DateTime::from_timestamp(t.seconds, t.nanos.max(0) as u32))
        .map(|t| t.format("%Y-%m-%d %H:%M:%SZ").to_string())
        .unwrap_or_else(|| "-".into())
}

/// "7d", "12h", "30m", "45s".
pub fn parse_duration(s: &str) -> anyhow::Result<chrono::Duration> {
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid duration {s:?}; use e.g. 30m, 12h or 7d"))?;
    anyhow::ensure!(n > 0, "duration must be positive");
    Ok(match unit {
        "s" => chrono::Duration::seconds(n),
        "m" => chrono::Duration::minutes(n),
        "h" => chrono::Duration::hours(n),
        "d" => chrono::Duration::days(n),
        _ => anyhow::bail!("invalid duration {s:?}; use e.g. 30m, 12h or 7d"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_aligns_and_json_uses_the_headers() {
        let mut t = Table::new(&["name", "last seen"]);
        t.row(vec!["web-01".into(), "now".into()]);
        t.row(vec!["db".into(), "-".into()]);
        assert_eq!(t.render(false), "name    last seen\nweb-01  now\ndb      -");
        let json: Value = serde_json::from_str(&t.render(true)).unwrap();
        assert_eq!(json[0]["last_seen"], "now");
    }

    #[test]
    fn durations_and_sizes() {
        assert_eq!(parse_duration("7d").unwrap(), chrono::Duration::days(7));
        assert_eq!(
            parse_duration("90m").unwrap(),
            chrono::Duration::minutes(90)
        );
        assert!(parse_duration("7").is_err());
        assert!(parse_duration("0h").is_err());
        assert!(parse_duration("").is_err());
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(16 * 1024 * 1024 * 1024), "16.0 GiB");
    }
}
