//! S2's re-scoped C010 gate: the REAL `rabs-wrap` binary consulting a
//! LIVE rabsd, measured end-to-end (process spawn + breaker read + UDS
//! connect + hello + consult + state write + exec), p95 < 10ms.
//! Enforcing under the release profile (the shipped artifact); loudly
//! measurement-only in debug — same calibration as the original C010.
#![cfg(unix)]

mod support;

use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use support::rabsd_bin;

const SLO_MS: f64 = 10.0;
const WARM_UPS: usize = 5;
const SAMPLES: usize = 60;

fn direct_sample(compiler: &std::path::Path) -> u128 {
    let start = Instant::now();
    let status = Command::new(compiler)
        .args(["--crate-name", "fx"])
        .status()
        .expect("direct compiler control");
    assert!(status.success(), "direct compiler control failed: {status}");
    start.elapsed().as_micros()
}

#[test]
fn wrapper_end_to_end_p95_under_10ms_against_live_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("rabsd.sock");
    let marker = dir.path().join("rabsd.boot");
    let mut daemon = Command::new(rabsd_bin())
        .env("RABS_SOCKET_PATH", &socket)
        .env("RABS_BOOT_MARKER", &marker)
        .env("RABS_STATE_DIR", dir.path().join("state"))
        .env("RABS_CONFIG", "/nonexistent-rabs-config")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rabsd");
    let deadline = Instant::now() + Duration::from_secs(10);
    while !socket.exists() {
        assert!(Instant::now() < deadline, "socket never appeared");
        std::thread::sleep(Duration::from_millis(10));
    }

    let fake = dir.path().join("fake-rustc");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let breaker = dir.path().join("breaker");

    // Warm-up (fs caches, first-connect costs), then measure. A failed
    // warm-up is not evidence that the measured daemon path is ready.
    for _ in 0..WARM_UPS {
        let status = Command::new(env!("CARGO_BIN_EXE_rabs-wrap"))
            .arg(&fake)
            .args(["--crate-name", "fx"])
            .env("RABS_BREAKER_FILE", &breaker)
            .env("RABS_SOCKET_PATH", &socket)
            .status()
            .expect("wrapper warm-up");
        assert!(status.success(), "wrapper warm-up failed: {status}");
    }
    // A contemporaneous unwrapped control distinguishes ordinary process
    // scheduling from wrapper/daemon work. Alternate order, keep EVERY sample,
    // and never subtract the control or use it to waive the absolute SLO.
    let mut direct_samples_us_in_order = Vec::with_capacity(SAMPLES);
    let mut samples: Vec<u128> = (0..SAMPLES)
        .map(|index| {
            if index % 2 == 0 {
                direct_samples_us_in_order.push(direct_sample(&fake));
            }
            let start = Instant::now();
            let status = Command::new(env!("CARGO_BIN_EXE_rabs-wrap"))
                .arg(&fake)
                .args(["--crate-name", "fx"])
                .env("RABS_BREAKER_FILE", &breaker)
                .env("RABS_SOCKET_PATH", &socket)
                .status()
                .expect("wrapper run");
            assert!(status.success());
            let elapsed = start.elapsed().as_micros();
            if index % 2 != 0 {
                direct_samples_us_in_order.push(direct_sample(&fake));
            }
            elapsed
        })
        .collect();
    // Retain observations after timing ends; a failed p95 alone cannot show
    // whether the whole run slowed down or only a few samples did.
    let samples_us_in_order = samples.clone();
    samples.sort_unstable();
    let p95_us = samples[(samples.len() * 95) / 100];
    let p95_ms = p95_us as f64 / 1_000.0;

    Command::new("kill")
        .args(["-TERM", &daemon.id().to_string()])
        .status()
        .unwrap();
    let daemon_status = daemon.wait().expect("reap owned performance daemon");
    assert!(
        daemon_status.success(),
        "daemon shutdown failed: {daemon_status}"
    );

    // A successful compiler exit alone can be fail-open passthrough. Require
    // one real receipt for EVERY warm-up and measured wrapper invocation, so
    // skipping a broken/busy daemon cannot produce a deceptively fast pass.
    let receipts = std::fs::read_to_string(dir.path().join("state/shadow-receipts.ndjson"))
        .expect("read actual daemon consult receipts");
    let mut confirmed_consults = 0;
    for line in receipts.lines() {
        let value: serde_json::Value = serde_json::from_str(line).expect("valid receipt");
        assert_eq!(value["kind"], "shadow-receipt");
        assert_eq!(value["decision"], "pass-through");
        assert_eq!(value["argv0"].as_str(), fake.to_str());
        confirmed_consults += 1;
    }
    assert_eq!(
        confirmed_consults,
        WARM_UPS + SAMPLES,
        "consults were skipped"
    );

    let enforcing = !cfg!(debug_assertions);
    let evidence = format!(
        "{{\"v\":1,\"suite\":\"perf/wrapper\",\"test\":\"rabs_wrap_end_to_end\",\
         \"p95_us\":{p95_us},\"SLO_ms\":{SLO_MS},\"profile\":\"{}\",\"gate\":\"{}\",\
         \"samples_us_in_order\":{samples_us_in_order:?},\
         \"direct_samples_us_in_order\":{direct_samples_us_in_order:?},\
         \"confirmed_consults\":{confirmed_consults}}}",
        if enforcing { "release" } else { "debug" },
        if enforcing {
            "enforcing"
        } else {
            "measurement-only"
        },
    );
    eprintln!("{evidence}");
    assert!(
        !enforcing || p95_ms < SLO_MS,
        "wrapper end-to-end SLO violated: {evidence}"
    );
}
