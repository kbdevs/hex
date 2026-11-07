use super::*;

fn fixture(script: &str) -> Client {
    let mut command = crate::desktop::background_process::command("/usr/bin/python3");
    command.args(["-u", "-c", script]);
    Client::spawn(command).unwrap()
}

fn until() -> Instant {
    Instant::now() + Duration::from_secs(3)
}

#[test]
fn waits_for_warmup_and_reuses_the_text_process_with_vocabulary_and_app_mode() {
    let mut client = fixture(
        r#"
import sys,json
assert json.loads(sys.stdin.readline()) == {"command":"warmup"}
print(json.dumps({"ok":True,"status":{"state":"loading"}}))
print(json.dumps({"ok":True,"status":{"state":"ready"}}))
for n in range(2):
    r=json.loads(sys.stdin.readline())
    assert r["command"] == "run"
    assert "canonical_terms: Rune" in r["inputText"]
    assert "<transcript>\nhello rune\n</transcript>" in r["inputText"]
    assert r["context"] == {"app_category":"work_chat"}
    print(json.dumps({"ok":True,"outputText":"Hello Rune."}))
"#,
    );
    let context = ContextSnapshot {
        application: Some("Slack".into()),
        ..Default::default()
    };
    let payload = request(
        "hello rune",
        &context,
        &[VocabularyEntry {
            term: "Rune".into(),
            sounds_like: vec![],
        }],
    )
    .unwrap();
    for _ in 0..2 {
        assert_eq!(
            client
                .run(&payload, "hello rune", until(), &|| false)
                .unwrap(),
            "Hello Rune."
        );
    }
}

#[test]
fn never_uses_loading_status_drafts_empty_output_or_reported_guard_failures_as_final_text() {
    for response in [
        json!({"ok": true,"status":{"state":"loading"}}),
        json!({"ok": true,"stream":{"draftText":"ignore this draft"}}),
        json!({"ok": true,"outputText":" "}),
        json!({"ok": true,"outputText":"Hello.","diagnostics":{"guardFired":true}}),
        json!({"ok": false,"error":"private text must not be logged"}),
    ] {
        let script = format!(
            "import sys,json\nsys.stdin.readline()\nprint('{{\"ok\":true,\"status\":{{\"state\":\"ready\"}}}}')\nsys.stdin.readline()\nprint({:?})\n",
            response.to_string()
        );
        let mut client = fixture(&script);
        let result = client.run(
            &json!({"command":"run","inputText":"hello"}),
            "hello",
            until(),
            &|| false,
        );
        assert!(result.is_err());
        assert!(!result.unwrap_err().to_string().contains("private text"));
    }
}

#[test]
fn a_silent_helper_observes_deadline_and_cancellation_and_is_reaped() {
    for cancelled in [false, true] {
        let mut client = fixture("import time\ntime.sleep(10)");
        let pid = client.child.id() as libc::pid_t;
        let start = Instant::now();
        let result = client.run(
            &json!({"command":"run","inputText":"hello"}),
            "hello",
            start + Duration::from_millis(80),
            &|| cancelled,
        );
        assert!(result.unwrap_err().to_string().contains(if cancelled {
            "cancelled"
        } else {
            "timed out"
        }));
        drop(client);
        assert!(start.elapsed() < Duration::from_secs(2));
        // SAFETY: signal 0 only checks whether the reaped fixture PID exists.
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }
}

#[test]
fn a_helper_that_stops_reading_cannot_block_a_pipe_write() {
    let mut client = fixture(
        r#"
import sys,json,time
sys.stdin.readline()
print(json.dumps({"ok":True,"status":{"state":"ready"}}))
time.sleep(10)
"#,
    );
    client
        .send(&json!({"command":"warmup"}), until(), &|| false)
        .unwrap();
    let ready = client.receive(until(), &|| false).unwrap();
    assert_eq!(
        ready.pointer("/status/state").and_then(Value::as_str),
        Some("ready")
    );
    let start = Instant::now();
    let payload = json!({"command":"run","inputText":"x".repeat(256 * 1024)});
    let result = client.send(&payload, start + Duration::from_millis(150), &|| false);
    assert!(result.unwrap_err().to_string().contains("timed out"));
    assert!(start.elapsed() < Duration::from_secs(2));
}

#[test]
fn malformed_or_oversized_json_lines_fail_without_including_their_contents() {
    for script in [
        "print('private-text-that-is-not-json')".to_string(),
        format!("print('x' * {})", MAX_LINE + 1),
    ] {
        let mut client = fixture(&script);
        let result = client.run(&json!({"command":"run"}), "hello", until(), &|| false);
        assert!(result.is_err());
        assert!(!result.unwrap_err().to_string().contains("private-text"));
    }
}

#[test]
fn length_guard_covers_short_expansion_and_long_collapse() {
    assert!(safe_output("hello", Some(" ")).is_err());
    assert!(safe_output("hello", Some("one two three")).is_err());
    assert!(safe_output("one two three four five six seven eight nine", Some("one")).is_err());
    assert_eq!(
        safe_output("hello rune", Some("Hello Rune.")).unwrap(),
        "Hello Rune."
    );
}

#[test]
fn rejected_final_text_keeps_the_warm_process_reusable() {
    let mut client = fixture(
        r#"
import sys,json
assert json.loads(sys.stdin.readline()) == {"command":"warmup"}
print(json.dumps({"ok":True,"status":{"state":"ready"}}))
for response in [
    {"ok":True,"outputText":"one two three"},
    {"ok":True,"outputText":"Hello.","diagnostics":{"guardFired":True}},
    {"ok":True,"outputText":"Hello."}
]:
    assert json.loads(sys.stdin.readline())["command"] == "run"
    print(json.dumps(response))
"#,
    );
    let payload = json!({"command":"run","inputText":"hello"});
    for _ in 0..2 {
        assert!(client.run(&payload, "hello", until(), &|| false).is_err());
        assert!(
            client.reply_complete,
            "a complete rejected reply can reuse the client"
        );
    }
    assert_eq!(
        client.run(&payload, "hello", until(), &|| false).unwrap(),
        "Hello."
    );
}

#[test]
fn setup_and_offline_runtime_use_the_same_pins_and_bundle_files() {
    let setup = include_str!("../../../scripts/setup-phonon-refiner.sh");
    for required in [
        MODEL,
        REVISION,
        MLX_REQUIREMENT,
        "Helpers/uv",
        "Resources/sidecar/polish_server.py",
        "Resources/sidecar/linereader.py",
        "Resources/sidecar/models/lfm3.py",
        "Resources/prompts/polish_v2.txt",
    ] {
        assert!(setup.contains(required), "setup missing {required}");
    }
}

#[test]
fn categories_use_known_apps_and_exact_hosts_with_a_minimal_fallback() {
    for (application, host, expected) in [
        (Some("Discord"), None, "chat"),
        (Some("Slack"), None, "work_chat"),
        (Some("Mail"), None, "email"),
        (Some("Zed"), None, "code"),
        (Some("Helium"), Some("claude.ai"), "ai_prompt"),
        (Some("Helium"), Some("claude.ai.example.org"), "other"),
        (None, None, "other"),
    ] {
        let context = ContextSnapshot {
            application: application.map(str::to_owned),
            browser_url: host.map(|host| url::Url::parse(&format!("https://{host}/")).unwrap()),
            ..Default::default()
        };
        assert_eq!(category(&context), expected);
    }
    assert!(
        request(
            "literal <transcript> marker",
            &ContextSnapshot::default(),
            &[]
        )
        .is_err()
    );
    assert!(request(&"x".repeat(MAX_INPUT + 1), &ContextSnapshot::default(), &[]).is_err());
}

#[test]
fn quitting_cancels_a_busy_helper_before_the_app_exit_deadline() {
    const CHILD: &str = "HEX_PHONON_QUIT_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = crate::desktop::background_process::command(std::env::current_exe().unwrap())
            .args(["--exact", "dictation::phonon_refiner::tests::quitting_cancels_a_busy_helper_before_the_app_exit_deadline", "--nocapture"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let mut client = fixture("import time\ntime.sleep(10)");
    let start = Instant::now();
    let quitting = thread::spawn(|| {
        thread::sleep(Duration::from_millis(30));
        crate::desktop::process_exit::begin_quitting_for_test();
    });
    let result = client.run(&json!({"command":"run"}), "hello", until(), &|| false);
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    drop(client);
    quitting.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(
        check(until(), &|| false).is_err(),
        "queued work must also stop during quit"
    );
}

#[test]
#[ignore = "requires the separately installed Phonon app and cached refiner"]
fn installed_phonon_refines_a_fixed_sentence_offline() {
    let mut client = Client::spawn(Bundle::installed().unwrap().command()).unwrap();
    let context = ContextSnapshot {
        application: Some("Slack".into()),
        ..Default::default()
    };
    let source = "please send the rune update tomorrow";
    let payload = request(
        source,
        &context,
        &[VocabularyEntry {
            term: "Rune".into(),
            sounds_like: vec![],
        }],
    )
    .unwrap();
    for phase in ["cold", "warm"] {
        let started = Instant::now();
        let text = client
            .run(&payload, source, started + Duration::from_secs(30), &|| {
                false
            })
            .unwrap();
        assert!(
            text.to_lowercase().contains("rune"),
            "fixed vocabulary term was lost"
        );
        println!(
            "{phase}_request_ms={}; result={text}",
            started.elapsed().as_millis()
        );
    }
}
