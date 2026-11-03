//! A custom harness lets this same test executable act as an owned child reader.
//! It exercises actual process/HTTP/file failures without a GPU, real model,
//! privileged disk quotas, physical unplugging, or claims of native inference.
use image::{DynamicImage, Rgb, RgbImage};
use notegen::{
    hardware, planning,
    storage::{self, Activity, Control},
    types::*,
    warm_runtime::WarmPool,
};
use serde::{Serialize, Serializer, ser::SerializeSeq};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, ExitCode},
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

fn option(args: &[String], name: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == name)
        .map(|pair| pair[1].clone())
}
fn request(socket: &mut TcpStream) -> std::io::Result<String> {
    socket.set_nonblocking(false)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut bytes = vec![];
    let mut buffer = [0u8; 4096];
    loop {
        let count = socket.read(&mut buffer)?;
        if count == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        bytes.extend_from_slice(&buffer[..count]);
        if bytes.len() > 8_000_000 {
            return Err(std::io::Error::other("test HTTP request bound exceeded"));
        }
        if let Some(split) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..split]);
            let length = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= split + 4 + length {
                return Ok(headers.lines().next().unwrap_or_default().to_string());
            }
        }
    }
}
fn reply(socket: &mut TcpStream, status: &str, body: &str) -> std::io::Result<()> {
    write!(
        socket,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    socket.flush()
}
fn reader_child(args: &[String]) -> std::io::Result<()> {
    let weights = PathBuf::from(option(args, "--model").expect("test reader model path"));
    let root = weights.parent().unwrap();
    let mode = fs::read_to_string(root.join("mode.txt"))?;
    let port: u16 = option(args, "--port")
        .expect("test reader port")
        .parse()
        .unwrap();
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        let mut socket = match listener.accept() {
            Ok((socket, _)) => socket,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => return Err(error),
        };
        let line = request(&mut socket)?;
        if line.starts_with("GET /health ") {
            reply(&mut socket, "200 OK", "{\"status\":\"ok\"}")?;
        } else if line.starts_with("POST /v1/chat/completions ") {
            fs::write(root.join("request-entered"), b"synthetic request received")?;
            match mode.as_str() {
                "process-loss" => std::process::exit(73),
                "simulated-gpu-loss" => reply(
                    &mut socket,
                    "500 Internal Server Error",
                    "{\"error\":\"CUDA error: simulated device loss; no actual GPU used\"}",
                )?,
                "transport-loss" | "cancel-in-flight" => {
                    // A valid fragment is not a completed reading. Advertise more
                    // bytes than sent, then either close or await cancellation.
                    let fragment =
                        "data: {\"choices\":[{\"delta\":{\"content\":\"1250 partial\"}}]}\n\n";
                    write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 100000\r\nConnection: close\r\n\r\n{fragment}"
                    )?;
                    socket.flush()?;
                    if mode == "cancel-in-flight" {
                        thread::sleep(Duration::from_secs(20));
                    }
                }
                _ => return Err(std::io::Error::other("unknown test failure mode")),
            }
        } else {
            reply(&mut socket, "404 Not Found", "{}")?;
        }
    }
    Ok(())
}
fn process_exists(pid: u32) -> bool {
    let pid = sysinfo::Pid::from_u32(pid);
    let mut system = sysinfo::System::new();
    system.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[pid]),
        true,
        sysinfo::ProcessRefreshKind::nothing().without_tasks(),
    );
    system.process(pid).is_some()
}
fn assert_stopped(pid: u32, endpoint: &str) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while process_exists(pid) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(!process_exists(pid), "owned test reader PID {pid} leaked");
    let endpoint = reqwest::Url::parse(endpoint).unwrap();
    let address = format!("127.0.0.1:{}", endpoint.port().unwrap())
        .parse()
        .unwrap();
    assert!(
        TcpStream::connect_timeout(&address, Duration::from_millis(200)).is_err(),
        "owned test reader listener leaked"
    );
}
fn managed_failure(mode: &str, expected_code: &str) {
    let root = tempfile::tempdir().unwrap();
    let weights = root.path().join("weights.gguf");
    let projector = root.path().join("projector.gguf");
    fs::write(&weights, b"GGUFsynthetic-failure-fixture").unwrap();
    fs::write(&projector, b"GGUFsynthetic-projector-fixture").unwrap();
    fs::write(root.path().join("mode.txt"), mode).unwrap();
    let mut model = planning::catalog().remove(0);
    model.id = "failure-fixture".into();
    model.name = "Synthetic failure process (not inference)".into();
    model.family = "synthetic".into();
    model.prompt = "structured".into();
    model.weights = weights.clone();
    model.projector = projector.clone();
    model.resident_mib = 32;
    model.weights_mib = 1;
    model.sha256.clear();
    let s = Settings {
        server: std::env::current_exe().unwrap(),
        backend: Backend::Cpu,
        automatic_models: false,
        model_ids: vec![model.id.clone()],
        reserve_ram_mib: 0,
        cpu_threads: 1,
        context: 8192,
        image_tokens: 1024,
        image_edge: 512,
        max_tokens: 512,
        request_timeout_secs: 5,
        ..Default::default()
    };
    let plan = planning::build(&hardware::detect(), &s, std::slice::from_ref(&model)).unwrap();
    assert_eq!(plan.models.len(), 1);
    let pool = WarmPool::new(Duration::from_secs(1)).unwrap();
    let c = Control::default();
    let cancellation = if mode == "cancel-in-flight" {
        let entered = root.path().join("request-entered");
        let cancellation = c.cancelled.clone();
        Some(thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            while !entered.exists() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            let received = entered.is_file();
            cancellation.store(true, Ordering::Release);
            received
        }))
    } else {
        None
    };
    let activity = Activity {
        path: Some(root.path().join("activity.jsonl")),
        callback: None,
    };
    let image = DynamicImage::ImageRgb8(RgbImage::from_pixel(100, 100, Rgb([255, 255, 255])));
    let mut owned = None;
    let started = Instant::now();
    let error = pool
        .with_reader(&model, &plan, &s, &c, &activity, root.path(), |reader| {
            owned = Some((
                reader.managed_pid().expect("test reader must be owned"),
                reader.endpoint.clone(),
            ));
            reader.read(
                &image,
                "",
                "Synthetic lifecycle failure test; not recognition accuracy",
                &s,
                &c,
                &activity,
                1,
            )
        })
        .unwrap_err();
    if let Some(cancellation) = cancellation {
        assert!(
            cancellation.join().unwrap(),
            "cancellation must occur after request admission"
        );
    }
    assert_eq!(
        error.downcast_ref::<Fault>().map(|fault| fault.code),
        Some(expected_code),
        "{error:#}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(12),
        "failure response exceeded the test bound"
    );
    assert!(root.path().join("request-entered").is_file());
    assert_eq!(
        pool.status().cached_pid,
        None,
        "failed reader must never enter the warm cache"
    );
    assert!(!pool.status().busy);
    let (pid, endpoint) = owned.unwrap();
    assert_stopped(pid, &endpoint);
    assert_eq!(fs::read(weights).unwrap(), b"GGUFsynthetic-failure-fixture");
    assert_eq!(
        fs::read(projector).unwrap(),
        b"GGUFsynthetic-projector-fixture"
    );
    let responses = root.path().join("responses");
    assert!(
        !responses.exists() || fs::read_dir(responses).unwrap().next().is_none(),
        "incomplete reply must not be saved as successful evidence"
    );
}
fn managed_process_exit_is_reaped() {
    managed_failure("process-loss", "NG200");
}
fn interrupted_stream_is_rejected_and_owned_process_reaped() {
    managed_failure("transport-loss", "NG200");
}
fn simulated_device_loss_drops_reader_without_claiming_a_gpu_test() {
    managed_failure("simulated-gpu-loss", "NG202");
}
fn cancellation_during_stream_closes_owned_process() {
    managed_failure("cancel-in-flight", "NG900");
}

struct DetachedStorage<'a> {
    attached: &'a Path,
    detached: &'a Path,
}
impl Serialize for DetachedStorage<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        fs::rename(self.attached, self.detached).map_err(serde::ser::Error::custom)?;
        "new complete value".serialize(serializer)
    }
}
fn storage_disappears_during_atomic_publication() {
    let root = tempfile::tempdir().unwrap();
    let attached = root.path().join("attached-storage");
    let detached = root.path().join("detached-storage");
    fs::create_dir(&attached).unwrap();
    let target = attached.join("working.json");
    let original = b"{\"preserved\":\"working output\"}";
    fs::write(&target, original).unwrap();
    assert!(
        storage::atomic_json(
            &target,
            &DetachedStorage {
                attached: &attached,
                detached: &detached
            }
        )
        .is_err()
    );
    assert!(!attached.exists());
    assert_eq!(fs::read(detached.join("working.json")).unwrap(), original);
    fs::rename(&detached, &attached).unwrap();
    assert_eq!(fs::read(&target).unwrap(), original);
    storage::atomic_json(&target, &"recovered complete value").unwrap();
    assert_eq!(
        storage::read_json::<String>(&target, 1000).unwrap(),
        "recovered complete value"
    );
}
fn storage_disappears_during_protected_publication() {
    let root = tempfile::tempdir().unwrap();
    let attached = root.path().join("attached-storage");
    let detached = root.path().join("detached-storage");
    fs::create_dir(&attached).unwrap();
    fs::write(attached.join("previous.docx"), b"previous output preserved").unwrap();
    let target = attached.join("new-output.json");
    assert!(
        storage::protected_json(
            &target,
            &DetachedStorage {
                attached: &attached,
                detached: &detached
            }
        )
        .is_err()
    );
    assert!(!attached.exists());
    assert!(!detached.join("new-output.json").exists());
    assert_eq!(
        fs::read(detached.join("previous.docx")).unwrap(),
        b"previous output preserved"
    );
    fs::rename(&detached, &attached).unwrap();
    storage::protected_json(&target, &"recovered complete value").unwrap();
    assert_eq!(
        storage::read_json::<String>(&target, 1000).unwrap(),
        "recovered complete value"
    );
    assert_eq!(
        fs::read(attached.join("previous.docx")).unwrap(),
        b"previous output preserved"
    );
}
struct AbruptSerialization;
impl Serialize for AbruptSerialization {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(2))?;
        sequence.serialize_element("first partially written element")?;
        std::process::exit(86)
    }
}
fn wait_owned(child: &mut Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned abrupt-write child exceeded its test bound");
        }
        thread::sleep(Duration::from_millis(10));
    }
}
fn abrupt_writer_process_exit_keeps_working_output_and_allows_recovery() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("working.json");
    let original = b"{\"verified\":\"old output\"}";
    fs::write(&output, original).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--abort-storage-write")
        .arg(&output)
        .spawn()
        .unwrap();
    let pid = child.id();
    assert_eq!(wait_owned(&mut child).code(), Some(86));
    assert!(!process_exists(pid));
    assert_eq!(fs::read(&output).unwrap(), original);
    assert!(
        fs::read_dir(root.path())
            .unwrap()
            .any(|entry| entry.unwrap().path() != output),
        "abrupt exit must leave the unpublished staging file inspectable"
    );
    storage::atomic_json(&output, &"recovered after abrupt writer exit").unwrap();
    assert_eq!(
        storage::read_json::<String>(&output, 1000).unwrap(),
        "recovered after abrupt writer exit"
    );
}
type Case = (&'static str, fn());
fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.iter().any(|arg| arg == "--list-devices") {
        println!("Synthetic CPU-only test process; no physical GPU exercised");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("Synthetic protocol fixture: build 12032, commit dde879bc4; not native inference");
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--model") {
        return if let Err(error) = reader_child(&args) {
            eprintln!("Synthetic child failed: {error}");
            ExitCode::FAILURE
        } else {
            ExitCode::SUCCESS
        };
    }
    if let Some(path) = option(&args, "--abort-storage-write") {
        let _ = storage::atomic_json(Path::new(&path), &AbruptSerialization);
        return ExitCode::FAILURE;
    }
    let cases: [Case; 7] = [
        (
            "managed_process_exit_is_reaped",
            managed_process_exit_is_reaped,
        ),
        (
            "interrupted_stream_is_rejected_and_owned_process_reaped",
            interrupted_stream_is_rejected_and_owned_process_reaped,
        ),
        (
            "simulated_device_loss_drops_reader_without_claiming_a_gpu_test",
            simulated_device_loss_drops_reader_without_claiming_a_gpu_test,
        ),
        (
            "cancellation_during_stream_closes_owned_process",
            cancellation_during_stream_closes_owned_process,
        ),
        (
            "storage_disappears_during_atomic_publication",
            storage_disappears_during_atomic_publication,
        ),
        (
            "storage_disappears_during_protected_publication",
            storage_disappears_during_protected_publication,
        ),
        (
            "abrupt_writer_process_exit_keeps_working_output_and_allows_recovery",
            abrupt_writer_process_exit_keeps_working_output_and_allows_recovery,
        ),
    ];
    if args.iter().any(|arg| arg == "--list") {
        for (name, _) in cases {
            println!("{name}: test");
        }
        return ExitCode::SUCCESS;
    }
    if args.iter().any(|arg| arg == "--ignored") {
        println!("0 ignored failure-injection cases");
        return ExitCode::SUCCESS;
    }
    let mut failed = 0;
    for (name, case) in cases {
        print!("test {name} ... ");
        if std::panic::catch_unwind(case).is_ok() {
            println!("ok");
        } else {
            failed += 1;
            println!("FAILED");
        }
    }
    println!(
        "Failure injection: {} passed; {failed} failed. Simulated backend faults; no physical GPU/storage damage or inference accuracy claim.",
        cases.len() - failed
    );
    if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
