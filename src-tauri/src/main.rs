// 릴리스 빌드에서 윈도우 콘솔 창을 띄우지 않는다
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Claude 가 MCP 서버로 띄울 때: `smash --mcp` (창 없이 stdin/stdout JSON-RPC)
    if std::env::args().any(|a| a == "--mcp") {
        smash_lib::mcp::run();
        return;
    }
    smash_lib::run();
}
