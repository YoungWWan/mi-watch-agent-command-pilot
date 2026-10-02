#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("小米手表 AI 指令助手 v{}", env!("CARGO_PKG_VERSION"));
        println!("Usage: agent-command-pilot [OPTIONS]\n");
        println!("Options:");
        println!("  --mcp             Run as Model Context Protocol (MCP) stdio server");
        println!("  --hook <EVENT>    Run as Agent Hook handler (e.g. --hook stop)");
        println!("  -v, --version     Print version information");
        println!("  -h, --help        Print help information");
        println!("  (no args)         Launch Desktop GUI application");
        return;
    }

    if args.iter().any(|a| a == "--version" || a == "-v") {
        println!("agent-command-pilot {}", env!("CARGO_PKG_VERSION"));
        return;
    }

    // Check if launched as an MCP server for Codex, Claude, Cursor, etc.
    if args.iter().any(|a| a == "--mcp") {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime for MCP");
        if let Err(e) = rt.block_on(agent_command_pilot_lib::cli::mcp::run_mcp_server()) {
            eprintln!("MCP Error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Check if launched as an agent Hook (e.g. Codex/Claude Stop event)
    if let Some(pos) = args.iter().position(|a| a == "--hook") {
        let hook_type = args.get(pos + 1).map(|s| s.as_str()).unwrap_or("stop");
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build tokio runtime for Hook");
        if let Err(e) = rt.block_on(agent_command_pilot_lib::cli::hooks::run_hook(hook_type)) {
            eprintln!("Hook Error: {e}");
            std::process::exit(1);
        }
        return;
    }

    // Default: Run full Tauri GUI application
    agent_command_pilot_lib::run();
}
