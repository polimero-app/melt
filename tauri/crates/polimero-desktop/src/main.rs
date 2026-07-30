use polimero_core::AppInfo;

#[tauri::command]
fn app_info() -> AppInfo {
    polimero_core::app_info()
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        std::process::exit(polimero_cli::run(
            &args,
            &mut std::io::stdout(),
            &mut std::io::stderr(),
        ));
    }

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_info])
        .run(tauri::generate_context!())
        .expect("error while running Polimero");
}
