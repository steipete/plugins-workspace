use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
    process,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};
use tauri::{Manager, Resource};
use tauri_plugin_updater::UpdaterExt;
use tauri_updater_install_proof::native::Event;

const MSI_MAGIC: &[u8] = &[0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];

struct RetainedResource(Arc<AtomicBool>);
impl Resource for RetainedResource {}
impl Drop for RetainedResource {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn write_json(path: &Path, value: serde_json::Value) {
    let mut file = fs::File::create(path).expect("create proof receipt");
    serde_json::to_writer(&mut file, &value).expect("write proof receipt");
    file.sync_all().expect("flush proof receipt");
}

fn option(args: &[String], name: &str) -> String {
    args.iter()
        .find_map(|arg| arg.strip_prefix(name))
        .expect("missing proof argument")
        .to_owned()
}

fn installer(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from(option(args, "--proof-root="));
    let prefix = option(args, "--proof-event-prefix=");
    let started = Event::open(&format!("{prefix}-started"))?;
    let release = Event::open(&format!("{prefix}-release"))?;
    write_json(
        &root.join("installer.json"),
        serde_json::json!({
            "pid": process::id(), "args": args,
        }),
    );
    started.signal()?;
    release.wait(30_000)?;
    Ok(())
}

fn run_app(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let mode = &args[1];
    let root = PathBuf::from(&args[2]);
    let prefix = &args[3];
    // Payloads are intentionally synthetic; no signature or download path is exercised.
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let endpoint = format!("http://{}/update", listener.local_addr()?);
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        let mut request = Vec::new();
        let mut buffer = [0_u8; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let count = stream.read(&mut buffer)?;
            if count == 0 || request.len() > 8192 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid metadata request",
                ));
            }
            request.extend_from_slice(&buffer[..count]);
        }
        let body = br#"{"version":"99.0.0","url":"https://example.invalid/unused","signature":"unused-install-boundary"}"#;
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n", body.len())?;
        stream.write_all(body)?;
        Ok(())
    });

    let mut context = tauri::test::mock_context(tauri::test::noop_assets());
    context.config_mut().plugins.0.insert(
        "updater".into(),
        serde_json::json!({
            "pubkey": "unused-install-boundary",
            "dangerousInsecureTransportProtocol": true,
            "windows": {"installMode":"quiet"},
        }),
    );
    let app = tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(context)?;
    let resource_dropped = Arc::new(AtomicBool::new(false));
    app.resources_table()
        .add(RetainedResource(resource_dropped.clone()));
    let hook_calls = Arc::new(AtomicUsize::new(0));
    let hook_counter = hook_calls.clone();
    let hook_resource = resource_dropped.clone();
    let hook_root = root.clone();
    let app_handle = app.handle().clone();
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint.parse()?])?
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .installer_args([
            format!("--proof-root=\"{}\"", root.display()),
            format!("--proof-event-prefix={prefix}"),
        ])
        .on_before_exit(move || {
            let count = hook_counter.fetch_add(1, Ordering::SeqCst) + 1;
            app_handle.cleanup_before_exit();
            write_json(
                &hook_root.join("cleanup.json"),
                serde_json::json!({
                    "hookCalls": count,
                    "resourceDropped": hook_resource.load(Ordering::SeqCst),
                }),
            );
        })
        .build()?;
    let update = tauri::async_runtime::block_on(updater.check())?
        .ok_or("metadata did not produce an Update")?
        .restart_after_install(false);
    server.join().expect("metadata server panicked")?;
    write_json(
        &root.join("checked.json"),
        serde_json::json!({"version":update.version}),
    );

    let bytes = if mode == "success-exe" {
        fs::read(std::env::current_exe()?)?
    } else {
        let system_root = root.join("synthetic-system-root");
        if mode == "success-msi" {
            fs::create_dir_all(system_root.join("System32"))?;
            fs::copy(
                std::env::current_exe()?,
                system_root.join("System32/msiexec.exe"),
            )?;
        }
        // Only this disposable child changes SYSTEMROOT; the failure path does not exist.
        std::env::set_var("SYSTEMROOT", system_root);
        MSI_MAGIC.to_vec()
    };
    let result = update.install(bytes);
    let error = match result {
        Err(tauri_plugin_updater::Error::Io(error)) => serde_json::json!({
            "kind":"io", "osError":error.raw_os_error(), "message":error.to_string(),
        }),
        Err(error) => serde_json::json!({"kind":"other", "message":error.to_string()}),
        Ok(()) => serde_json::json!({"kind":"unexpected-success-return"}),
    };
    write_json(
        &root.join("returned.json"),
        serde_json::json!({
            "error":error,
            "hookCalls":hook_calls.load(Ordering::SeqCst),
            "resourceDropped":resource_dropped.load(Ordering::SeqCst),
        }),
    );
    // Preserve the observed post-return state: process::exit deliberately does not drop the app.
    process::exit(42);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args.iter().any(|arg| arg == "/UPDATE" || arg == "/i") {
        installer(&args)
    } else if args.first().map(String::as_str) == Some("--probe-app") && args.len() == 4 {
        run_app(&args)
    } else {
        Err("use the Windows integration test runner".into())
    };
    if let Err(error) = result {
        eprintln!("updater install proof setup/runtime failure: {error}");
        process::exit(43);
    }
}
