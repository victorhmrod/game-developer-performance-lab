use serde::Serialize;
use std::{error::Error, io::Read};

#[derive(Debug, Serialize)]
pub struct Distribution {
    pub count: usize,
    pub mean: f64,
    pub median: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub sample_standard_deviation: Option<f64>,
    pub coefficient_of_variation: Option<f64>,
    pub tukey_outlier_count: usize,
}

/// Linear interpolation at index (n - 1) * p, after sorting ascending.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let index = (sorted.len() - 1) as f64 * p;
    let lower = index.floor() as usize;
    let upper = index.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (index - lower as f64)
}

pub fn summarize(values: &[f64]) -> Result<Distribution, &'static str> {
    if values.is_empty() || values.iter().any(|x| !x.is_finite() || *x <= 0.0) {
        return Err("samples must be non-empty, finite, and positive");
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    if !mean.is_finite() {
        return Err("sample mean overflow");
    }
    let deviation = if sorted.len() > 1 {
        let variance =
            sorted.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (sorted.len() - 1) as f64;
        if !variance.is_finite() {
            return Err("sample variance overflow");
        }
        Some(variance.sqrt())
    } else {
        None
    };
    let q1 = percentile(&sorted, 0.25);
    let q3 = percentile(&sorted, 0.75);
    let iqr = q3 - q1;
    Ok(Distribution {
        count: sorted.len(),
        mean,
        median: percentile(&sorted, 0.5),
        p50: percentile(&sorted, 0.5),
        p95: percentile(&sorted, 0.95),
        p99: percentile(&sorted, 0.99),
        sample_standard_deviation: deviation,
        coefficient_of_variation: deviation.map(|sd| sd / mean),
        tukey_outlier_count: sorted
            .iter()
            .filter(|x| **x < q1 - 1.5 * iqr || **x > q3 + 1.5 * iqr)
            .count(),
    })
}

#[derive(Debug, Serialize)]
pub struct FrameAnalysis {
    pub schema_version: u32,
    pub source: &'static str,
    pub process_id: u32,
    pub swap_chain: String,
    pub selected_rows: usize,
    pub unavailable_intervals: usize,
    pub frame_times_ms: Vec<f64>,
    pub distribution_ms: Distribution,
    pub average_fps: f64,
    pub median_fps: f64,
    pub one_percent_low_fps: Option<f64>,
    pub point_one_percent_low_fps: Option<f64>,
    pub stutter_threshold_ms: f64,
    pub stutter_count: usize,
    pub stutter_excess_ms: f64,
    pub capture_coverage: Option<f64>,
    pub lost_events: Option<u64>,
    pub warnings: Vec<&'static str>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SwapChainActivity {
    pub address: String,
    pub rows: usize,
}

/// Identify streams belonging to one PID without pooling them.
pub fn presentmon_swap_chains(
    reader: impl Read,
    pid: u32,
) -> Result<Vec<SwapChainActivity>, Box<dyn Error>> {
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv.headers()?.clone();
    let process = headers
        .iter()
        .position(|v| v == "ProcessID")
        .ok_or("missing PresentMon column: ProcessID")?;
    let swap = headers
        .iter()
        .position(|v| v == "SwapChainAddress")
        .ok_or("missing PresentMon column: SwapChainAddress")?;
    let mut streams = std::collections::BTreeMap::<String, usize>::new();
    for row in csv.records() {
        let row = row?;
        let row_pid: u32 = row[process].parse().map_err(|_| "invalid ProcessID")?;
        if row_pid != pid {
            continue;
        }
        let address = row[swap].trim().to_ascii_lowercase();
        if address.is_empty() {
            return Err("empty swap chain address".into());
        }
        *streams.entry(address).or_default() += 1;
    }
    Ok(streams
        .into_iter()
        .map(|(address, rows)| SwapChainActivity { address, rows })
        .collect())
}

/// Import PresentMon CSV output. No capture completeness is inferred from CSV rows.
pub fn analyze_presentmon(
    reader: impl Read,
    pid: u32,
    swap_chain: &str,
    stutter_ms: f64,
) -> Result<FrameAnalysis, Box<dyn Error>> {
    if !stutter_ms.is_finite() || stutter_ms <= 0.0 {
        return Err("stutter threshold must be finite and positive".into());
    }
    let mut csv = csv::Reader::from_reader(reader);
    let headers = csv.headers()?.clone();
    let column = |name: &str| {
        headers
            .iter()
            .position(|value| value == name)
            .ok_or_else(|| format!("missing PresentMon column: {name}"))
    };
    let process_col = column("ProcessID")?;
    let swap_col = column("SwapChainAddress")?;
    let interval_col = column("MsBetweenDisplayChange")?;
    let mut values = Vec::new();
    let mut rows = 0;
    let mut unavailable = 0;
    for record in csv.records() {
        let record = record?;
        let row_pid: u32 = record[process_col]
            .parse()
            .map_err(|_| "invalid ProcessID")?;
        if row_pid != pid || !record[swap_col].eq_ignore_ascii_case(swap_chain) {
            continue;
        }
        rows += 1;
        let value = record[interval_col].trim();
        if value.is_empty() || value.eq_ignore_ascii_case("NA") {
            unavailable += 1;
            continue;
        }
        let value: f64 = value.parse().map_err(|_| "invalid display interval")?;
        if !value.is_finite() || value <= 0.0 {
            return Err("display intervals must be finite and positive".into());
        }
        values.push(value);
    }
    let distribution = summarize(&values)?;
    let mut sorted = values.clone();
    sorted.sort_by(f64::total_cmp);
    let tail_fps =
        |count: usize| 1000.0 * count as f64 / sorted[sorted.len() - count..].iter().sum::<f64>();
    let stutter_count = values.iter().filter(|x| **x > stutter_ms).count();
    let stutter_excess_ms = values
        .iter()
        .filter(|x| **x > stutter_ms)
        .map(|x| x - stutter_ms)
        .sum();
    Ok(FrameAnalysis {
        schema_version: 1,
        source: "PresentMon MsBetweenDisplayChange CSV",
        process_id: pid,
        swap_chain: swap_chain.into(),
        selected_rows: rows,
        unavailable_intervals: unavailable,
        average_fps: 1000.0 / distribution.mean,
        median_fps: 1000.0 / distribution.median,
        one_percent_low_fps: (sorted.len() >= 100).then(|| tail_fps(sorted.len().div_ceil(100))),
        point_one_percent_low_fps: (sorted.len() >= 1000)
            .then(|| tail_fps(sorted.len().div_ceil(1000))),
        distribution_ms: distribution,
        frame_times_ms: values,
        stutter_threshold_ms: stutter_ms,
        stutter_count,
        stutter_excess_ms,
        capture_coverage: None,
        lost_events: None,
        warnings: vec![
            "CSV alone does not establish capture coverage or ETW event loss",
            "Unavailable intervals are not necessarily dropped frames",
            "Displayed intervals are not simulation or Present() intervals",
            "Capture provenance and process identity must be verified independently",
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distribution_definitions_and_outliers() {
        let d = summarize(&[1., 2., 3., 4., 100.]).unwrap();
        assert_eq!(d.mean, 22.);
        assert_eq!(d.median, 3.);
        assert!((d.p95 - 80.8).abs() < 1e-10);
        assert_eq!(d.tukey_outlier_count, 1);
        assert!(
            (summarize(&[1., 2., 3.])
                .unwrap()
                .sample_standard_deviation
                .unwrap()
                - 1.)
                .abs()
                < 1e-10
        );
    }
    #[test]
    fn rejects_invalid_samples() {
        for values in [
            &[][..],
            &[0.][..],
            &[-1.][..],
            &[f64::NAN][..],
            &[f64::INFINITY][..],
        ] {
            assert!(summarize(values).is_err());
        }
        assert!(
            summarize(&[1.])
                .unwrap()
                .sample_standard_deviation
                .is_none()
        );
    }
    #[test]
    fn imports_only_selected_stream_and_reports_missing() {
        let csv = b"ProcessID,SwapChainAddress,MsBetweenDisplayChange\n1,0xA,10\n1,0xA,NA\n1,0xA,20\n1,0xB,100\n2,0xA,100\n";
        let result = analyze_presentmon(&csv[..], 1, "0xa", 15.).unwrap();
        assert_eq!(result.selected_rows, 3);
        assert_eq!(result.unavailable_intervals, 1);
        assert_eq!(result.frame_times_ms, vec![10., 20.]);
        assert!((result.average_fps - 1000. / 15.).abs() < 1e-9);
        assert_eq!(result.stutter_count, 1);
        assert_eq!(result.stutter_excess_ms, 5.);
        assert!(result.one_percent_low_fps.is_none());
        assert!(result.capture_coverage.is_none());
    }
    #[test]
    fn rejects_missing_columns_invalid_values_and_empty_selection() {
        assert!(analyze_presentmon(&b"ProcessID\n1\n"[..], 1, "a", 20.).is_err());
        assert!(
            analyze_presentmon(
                &b"ProcessID,SwapChainAddress,MsBetweenDisplayChange\n1,a,0\n"[..],
                1,
                "a",
                20.
            )
            .is_err()
        );
        assert!(
            analyze_presentmon(
                &b"ProcessID,SwapChainAddress,MsBetweenDisplayChange\n1,a,10\n"[..],
                2,
                "a",
                20.
            )
            .is_err()
        );
    }
    #[test]
    fn discovers_streams_for_only_the_selected_process() {
        let csv = b"ProcessID,SwapChainAddress,MsBetweenDisplayChange\n1,0xA,10\n1,0xa,20\n1,0xB,30\n2,0xC,20\n";
        assert_eq!(
            presentmon_swap_chains(&csv[..], 1).unwrap(),
            vec![
                SwapChainActivity {
                    address: "0xa".into(),
                    rows: 2
                },
                SwapChainActivity {
                    address: "0xb".into(),
                    rows: 1
                },
            ]
        );
    }
    #[test]
    fn low_fps_uses_slowest_frame_tail() {
        let mut csv = String::from("ProcessID,SwapChainAddress,MsBetweenDisplayChange\n");
        for _ in 0..999 {
            csv.push_str("1,a,10\n");
        }
        csv.push_str("1,a,100\n");
        let r = analyze_presentmon(csv.as_bytes(), 1, "a", 50.).unwrap();
        assert!((r.one_percent_low_fps.unwrap() - 1000. * 10. / 190.).abs() < 1e-9);
        assert_eq!(r.point_one_percent_low_fps.unwrap(), 10.);
    }
}
