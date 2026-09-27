//! Headless exercise of Ninfer::start() (safe while the engine is already
//! up: keepalive exists -> transient `systemctl start` no-op). Prints the
//! keepalive session count before/after.
use dock_core::Ninfer;

#[tokio::main]
async fn main() {
    let n = Ninfer::default();
    let count = || {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='wsl.exe'\" | Where-Object { $_.CommandLine -like '*tail -f /dev/null*' } | Measure-Object | Select-Object -ExpandProperty Count",
            ])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    };
    println!("keepalive sessions before: {}", count());
    match n.start().await {
        Ok(()) => println!("start: OK"),
        Err(e) => {
            println!("start: ERR {e}");
            std::process::exit(1);
        }
    }
    println!("health: {}", n.health().await);
    println!("keepalive sessions after: {}", count());
}
