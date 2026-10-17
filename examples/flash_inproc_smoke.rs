//! Start the Flash host inside this process and measure the direct path.
//!
//! Local movie (no session):
//!   cargo run --example flash_inproc_smoke -- --movie C:\path\movie.swf
//! Authenticated game: pipe the six bootstrap records (movie URL, base URL,
//! SID, FlashVars, width, height) on stdin, exactly as for
//! `xenomit_flash_client.exe --launch-stdin`. Never pass them as arguments.
//!   <bootstrap> | cargo run --example flash_inproc_smoke -- --stdin --backend native
//!
//! Options: --backend native|cached, --visible, --calls N (frame-call
//! benchmark), --seconds N (keep running), --restart (stop and start again),
//! --compare-pipe PID (run the same benchmark against an exe host).
//! Set XENOMIT_FLASH_DLL when the DLL is not beside this executable or in a
//! sibling xenomit-flash-client/build directory.

use std::io::BufRead;
use std::time::{Duration, Instant};

use xenomit_game_api::{
    FlashInProcClient, FlashInProcConfig, FunctionCall, GameClient, LiveFrame, WindowCommand,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let number =
        |name: &str, default: usize| value(name).and_then(|v| v.parse().ok()).unwrap_or(default);

    let mut config = FlashInProcConfig {
        backend: value("--backend").unwrap_or_else(|| "cached".into()),
        hidden: !flag("--visible"),
        hidden_draw_bootstrap: !flag("--visible"),
        ..FlashInProcConfig::default()
    };
    if flag("--stdin") {
        let mut lines = std::io::stdin().lock().lines();
        let mut next = |what: &str| -> Result<String, String> {
            match lines.next() {
                Some(Ok(line)) if !line.is_empty() => Ok(line.trim_end_matches('\r').to_owned()),
                _ => Err(format!("missing bootstrap record: {what}")),
            }
        };
        config.movie = next("movie")?;
        config.base_url = next("base url")?;
        config.sid = next("sid")?;
        config.flash_vars = next("flash vars")?;
        config.width = next("width")?.parse().map_err(|_| "invalid width")?;
        config.height = next("height")?.parse().map_err(|_| "invalid height")?;
    } else {
        config.movie = value("--movie").ok_or("pass --movie <swf> or --stdin")?;
    }

    let started = Instant::now();
    let client = FlashInProcClient::start(&config).map_err(|e| e.to_string())?;
    println!(
        "started in {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    wait_for_bridge(&client)?;
    report(&client, number("--calls", 100))?;

    if flag("--restart") {
        drop(client);
        let restarted = Instant::now();
        let client = FlashInProcClient::start(&config).map_err(|e| e.to_string())?;
        println!(
            "restarted in {:.0} ms",
            restarted.elapsed().as_secs_f64() * 1000.0
        );
        wait_for_bridge(&client)?;
        report(&client, 20)?;
        keep_running(&client, number("--seconds", 0))?;
    } else {
        keep_running(&client, number("--seconds", 0))?;
    }

    if let Some(pid) = value("--compare-pipe") {
        let pid: u32 = pid.parse().map_err(|_| "invalid pid")?;
        let pipe = xenomit_game_api::connect_flash(pid).map_err(|e| e.to_string())?;
        println!("-- exe host over the pipe (pid {pid})");
        bench("status()", 200, || pipe.status().map(|_| ()));
        bench("frame call (FINVOKE)", number("--calls", 100), || {
            pipe.call_function(&features()).map(|_| ())
        });
        contention(pipe.as_ref());
    }
    Ok(())
}

fn features() -> FunctionCall {
    FunctionCall::new("xenomit", "clientFeatures").in_frame()
}

fn wait_for_bridge(client: &FlashInProcClient) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if client.ping().is_ok() {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("the bridge did not answer within 60 s".into())
}

fn report(client: &FlashInProcClient, calls: usize) -> Result<(), String> {
    let status = client.status().map_err(|e| e.to_string())?;
    println!(
        "status: backend={} connected={} ready={}",
        status.backend, status.connected, status.ready
    );
    for key in [
        "fps_milli",
        "vfs_hits",
        "net_reads",
        "game_socket",
        "game_recv",
        "cpu_pct",
        "mem_ws_bytes",
    ] {
        println!(
            "  {key}={}",
            status.details.get(key).cloned().unwrap_or_default()
        );
    }
    match client.capabilities() {
        Ok(caps) => println!("caps: {caps:?}"),
        Err(error) => println!("caps unavailable: {error}"),
    }
    client
        .set_window(WindowCommand::MaxFps { fps: 60 })
        .map_err(|e| e.to_string())?;

    println!("-- in-process host");
    bench("status()", 1000, || client.status().map(|_| ()));
    let mut frame = LiveFrame::default();
    if status.details.get("native").and_then(|v| v.as_u64()) == Some(1) {
        bench("read_live_frame()", 1000, || {
            client.read_live_frame(&mut frame);
            Ok(())
        });
    }
    bench("immediate call (INVOKE)", calls, || {
        client
            .call_function(&FunctionCall::new("xenomit", "clientFeatures"))
            .map(|_| ())
    });
    bench("frame call (FINVOKE)", calls, || {
        client.call_function(&features()).map(|_| ())
    });
    contention(client);
    Ok(())
}

/// Status reads while another thread keeps frame calls in flight: the
/// bot's worker acting while the GUI polls.
fn contention(client: &dyn GameClient) {
    let stop = std::sync::atomic::AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let _ = client.call_function(&features());
            }
        });
        std::thread::sleep(Duration::from_millis(50));
        bench("status() during frame calls", 200, || {
            client.status().map(|_| ())
        });
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    });
}

fn keep_running(client: &FlashInProcClient, seconds: usize) -> Result<(), String> {
    for _ in 0..seconds {
        std::thread::sleep(Duration::from_secs(1));
        let status = client.status().map_err(|e| e.to_string())?;
        println!(
            "ready={} fps_milli={} game_recv={} game_recv_age_ms={}",
            status.ready,
            status.details["fps_milli"],
            status.details["game_recv"],
            status.details["game_recv_age_ms"]
        );
    }
    Ok(())
}

fn bench(name: &str, count: usize, mut call: impl FnMut() -> xenomit_game_api::Result<()>) {
    let mut times = Vec::with_capacity(count);
    let mut failures = 0;
    for _ in 0..count.max(1) {
        let start = Instant::now();
        if call().is_err() {
            failures += 1;
        }
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let pick = |q: f64| times[((times.len() - 1) as f64 * q) as usize];
    println!(
        "  {name:<26} n={:<5} median {:>8.3} ms  p95 {:>8.3} ms  failures {failures}",
        times.len(),
        pick(0.5),
        pick(0.95)
    );
}
