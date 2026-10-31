# Deploy to VPS (192.168.0.47)

1. Build release: `cargo build -p zn-server --release`
2. Copy `target/release/zn-server.exe` to `C:\zona\` on VPS.
3. Copy `zona.db` (if exists) or let server create it.
4. Run `deploy\watchdog.ps1` (or set up Task Scheduler to run at startup).
5. Ensure ports 7777 (TCP) and 7778 (UDP) are open in firewall.
6. Test: `cargo run -p zn-client -- --server 192.168.0.47`
