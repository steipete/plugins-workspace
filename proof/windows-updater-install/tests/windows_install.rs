use std::{
    fs, io,
    os::windows::io::AsRawHandle,
    path::Path,
    process::{Child, Command, Stdio},
};
use tauri_updater_install_proof::native::{wait_handle, Event};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{OpenProcess, SYNCHRONIZATION_SYNCHRONIZE},
};

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("missing native proof receipt")).unwrap()
}

fn wait_child(child: &mut Child) -> io::Result<std::process::ExitStatus> {
    if let Err(error) = wait_handle(child.as_raw_handle(), 30_000) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    child.wait()
}

fn release_and_join_installer(
    root: &Path,
    started: &Event,
    release: &Event,
) -> io::Result<serde_json::Value> {
    let started_result = started.wait(30_000);
    let installer: io::Result<serde_json::Value> = fs::read(root.join("installer.json"))
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(io::Error::other));
    let process = installer
        .as_ref()
        .ok()
        .and_then(|receipt| receipt["pid"].as_u64())
        .and_then(|pid| u32::try_from(pid).ok())
        .ok_or_else(|| io::Error::other("missing installer PID"))
        .and_then(|pid| {
            // Acquire the process handle before releasing the helper, preventing PID reuse.
            let handle = unsafe { OpenProcess(SYNCHRONIZATION_SYNCHRONIZE, 0, pid) };
            if handle.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(handle)
            }
        });
    // Release and join before propagating errors or checking behavior receipts.
    let released = release.signal();
    let joined = process.and_then(|handle| {
        let result = wait_handle(handle, 30_000);
        unsafe {
            CloseHandle(handle);
        }
        result
    });
    started_result?;
    released?;
    joined?;
    installer
}

fn exercise(mode: &str) {
    let evidence =
        std::env::var_os("UPDATER_INSTALL_PROOF_EVIDENCE").expect("set proof evidence directory");
    let case_root = Path::new(&evidence).join(mode);
    fs::create_dir_all(&case_root).unwrap();
    let work = tempfile::Builder::new()
        .prefix("owned-")
        .tempdir_in(&case_root)
        .unwrap();
    let root = work.path();
    let temp = root.join("temp");
    fs::create_dir(&temp).unwrap();
    let prefix = format!(
        "Local\\tauri-updater-proof-{}-{}",
        std::process::id(),
        root.file_name().unwrap().to_string_lossy()
    );
    let started = Event::create(&format!("{prefix}-started")).unwrap();
    let release = Event::create(&format!("{prefix}-release")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_tauri-updater-install-proof"))
        .args(["--probe-app", mode])
        .arg(root)
        .arg(&prefix)
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .stdin(Stdio::null())
        .stdout(fs::File::create(case_root.join("stdout.log")).unwrap())
        .stderr(fs::File::create(case_root.join("stderr.log")).unwrap())
        .spawn()
        .unwrap();
    let installer_result = if mode == "failed-msi-launch" {
        None
    } else {
        Some(release_and_join_installer(root, &started, &release))
    };
    let child_result = wait_child(&mut child);
    for receipt in [
        "checked.json",
        "returned.json",
        "cleanup.json",
        "installer.json",
    ] {
        let source = root.join(receipt);
        if source.exists() {
            fs::copy(&source, case_root.join(receipt)).expect("preserve native proof receipt");
        }
    }
    let status = child_result.expect("updater child did not finish");
    let checked = read_json(&root.join("checked.json"));
    assert_eq!(checked["version"], "99.0.0");

    let outcome = if mode == "failed-msi-launch" {
        assert_eq!(
            status.code(),
            Some(42),
            "install must return an error without exiting the app"
        );
        let returned = read_json(&root.join("returned.json"));
        fs::write(
            case_root.join("outcome.json"),
            serde_json::to_vec_pretty(&returned).unwrap(),
        )
        .unwrap();
        assert_eq!(
            returned["error"]["kind"], "io",
            "the fixture must reach native launch, not format rejection"
        );
        assert_eq!(
            returned["hookCalls"], 0,
            "cleanup hook ran before installer launch failed"
        );
        assert_eq!(
            returned["resourceDropped"], false,
            "failed launch destroyed the application's resource"
        );
        assert!(!root.join("cleanup.json").exists());
        returned
    } else {
        let return_details = fs::read_to_string(root.join("returned.json"))
            .unwrap_or_else(|error| format!("no return receipt: {error}"));
        assert_eq!(
            status.code(),
            Some(0),
            "successful install must use process::exit(0); {return_details}"
        );
        assert!(
            !root.join("returned.json").exists(),
            "successful Windows install returned: {return_details}"
        );
        let installer = installer_result
            .unwrap()
            .expect("real ShellExecuteW installer did not start and finish");
        let cleanup = read_json(&root.join("cleanup.json"));
        assert_eq!(cleanup["hookCalls"], 1);
        assert_eq!(cleanup["resourceDropped"], true);
        let args = installer["args"].as_array().unwrap();
        let expected_flag = if mode == "success-msi" {
            "/i"
        } else {
            "/UPDATE"
        };
        assert!(args.iter().any(|arg| arg.as_str() == Some(expected_flag)));
        serde_json::json!({"cleanup":cleanup,"installer":installer,"exitCode":status.code()})
    };
    fs::write(
        case_root.join("outcome.json"),
        serde_json::to_vec_pretty(&outcome).unwrap(),
    )
    .unwrap();
}

#[test]
fn failed_installer_keeps_application_resources() {
    exercise("failed-msi-launch");
}

#[test]
fn successful_nsis_launch_cleans_up_and_exits() {
    exercise("success-exe");
}

#[test]
fn successful_msi_launch_cleans_up_and_exits() {
    exercise("success-msi");
}
