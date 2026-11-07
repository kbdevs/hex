use super::*;
use crate::history::HistoryDraft;

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    archive: PathBuf,
    support: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "hex-superwhisper-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(root.join("source/recordings")).unwrap();
        let fixture = Self {
            source: root.join("source"),
            archive: root.join("archive"),
            support: root.join("hex"),
            root,
        };
        write_private(
            &fixture.support.join("settings.json"),
            br#"{"history_retention":"forever"}"#,
        )
        .unwrap();
        fixture
    }

    fn record(&self, id: &str, meta: &Value, audio: &str) {
        let folder = self.source.join("recordings").join(id);
        write_private(
            &folder.join("meta.json"),
            &serde_json::to_vec(meta).unwrap(),
        )
        .unwrap();
        write_private(&folder.join(audio), b"original audio bytes").unwrap();
    }

    fn import(&self, dry: bool, no_audio: bool) -> Result<Report> {
        import_into(&self.source, &self.archive, &self.support, dry, no_audio)
    }

    fn archive_path(&self) -> PathBuf {
        source_archive(&self.source, &self.archive).unwrap().0
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn imports_original_text_speakers_titles_audio_and_templates_without_duplicates() {
    let fixture = Fixture::new();
    let native_path = fixture.support.join("history.json");
    let mut native = HistoryStore::open(native_path.clone(), HistoryRetention::Forever, 0);
    let native_id = native
        .record(
            HistoryDraft {
                kind: HistoryKind::Dictation,
                raw_text: "native".into(),
                final_text: "native".into(),
                application: None,
                processing: None,
                audio_ms: 1,
                inference_ms: 1,
                total_ms: 2,
                recording: None,
                model: None,
            },
            crate::history::now_ms(),
        )
        .unwrap()
        .unwrap();
    let before = fs::read(&native_path).unwrap();
    fixture.record(
        "1730000000",
        &serde_json::json!({
            "datetime": "2024-10-27T03:33:20",
            "duration": 4200,
            "rawResult": "raw\ntranscript",
            "result": "transcript",
            "llmResult": "Edited.\n\nTwo paragraphs.",
            "processingTime": 10,
            "languageModelProcessingTime": 20,
            "modeName": "Pro",
            "promptContext": {"applicationContext":{"name":"Editor"}}
        }),
        "output.wav",
    );
    fixture.record(
        "1730000100",
        &serde_json::json!({
            "datetime": "2024-10-27T03:35:00Z",
            "kind": "meeting",
            "title": "Planning meeting",
            "duration": 9000,
            "result": "Hello there. Agreed.",
            "llmResult": "# Decisions\n\n- Ship on Friday.",
            "speakers": [{"number":0, "name":"Alex"},{"number":1, "name":"Sam"}],
            "segments": [
                {"text":"Hello", "start":1.0,"end":2.0,"speaker":0},
                {"text":"there.", "start":2.0,"end":3.0,"speaker":0},
                {"text":"Agreed.", "start":4.0,"end":5.0,"speaker":1}
            ]
        }),
        "output.ogg",
    );
    write_private(
        &fixture.source.join("meeting-templates/actions.json"),
        br#"{"key":"actions","name":"Actions","prompt":"List tasks.","isStarred":true,"version":1}"#,
    )
    .unwrap();
    // A record with missing metadata still has its original audio preserved.
    write_private(
        &fixture.source.join("recordings/1730000200/output.wav"),
        b"orphan audio",
    )
    .unwrap();

    let dry = fixture.import(true, false).unwrap();
    assert_eq!(
        (
            dry.history_candidates,
            dry.meetings_found,
            dry.templates_found
        ),
        (1, 1, 1)
    );
    assert_eq!(
        (dry.history_added, dry.meetings_added, dry.files_copied),
        (0, 0, 0)
    );
    assert!(!fixture.archive.exists());
    assert_eq!(fs::read(&native_path).unwrap(), before);

    let report = fixture.import(false, false).unwrap();
    assert_eq!(
        (
            report.history_added,
            report.meetings_added,
            report.templates_added
        ),
        (1, 1, 1)
    );
    assert_eq!(report.audio_files_copied, 3, "WAV, OGG, and orphan audio");
    assert_eq!(report.warnings.len(), 1);
    let saved = crate::history::read_snapshot(&native_path).unwrap();
    assert_eq!(saved.len(), 2);
    let imported = saved.iter().find(|entry| entry.source.is_some()).unwrap();
    assert_eq!(imported.timestamp_ms, 1_730_000_000_000);
    assert_eq!(imported.raw_text, "raw\ntranscript");
    assert_eq!(imported.final_text, "Edited.\n\nTwo paragraphs.");
    assert_eq!(imported.application.as_deref(), Some("Editor"));
    assert_eq!((imported.inference_ms, imported.total_ms), (10, 30));
    let reopened = HistoryStore::open(native_path.clone(), HistoryRetention::Forever, 0);
    assert_eq!(
        reopened.entries().next().unwrap().id,
        native_id,
        "newest stays first"
    );
    let meeting = fixture.support.join("meetings").join(format!(
        "superwhisper-{}-1730000100",
        report.source_namespace.as_ref().unwrap()
    ));
    let manifest: MeetingManifest = read_json(&meeting.join("manifest.json")).unwrap();
    assert_eq!(manifest.title, "Planning meeting");
    assert_eq!(
        manifest.notes.as_ref().unwrap().summary,
        "# Decisions\n\n- Ship on Friday."
    );
    let notes = crate::meeting::notes_markdown(manifest.notes.as_ref().unwrap());
    assert!(notes.contains("Imported from Superwhisper"));
    assert!(!notes.contains("Apple Intelligence"));
    let turns = transcript_entries(
        &read_json::<Meta>(&fixture.source.join("recordings/1730000100/meta.json")).unwrap(),
    );
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].text, "[Alex] Hello there.");
    assert_eq!(turns[1].text, "[Sam] Agreed.");
    let template: crate::meeting_templates::MeetingTemplate =
        read_json(&fixture.support.join("meeting-templates/actions.json")).unwrap();
    assert!(template.is_starred);
    for name in [
        "1730000000/output.wav",
        "1730000100/output.ogg",
        "1730000200/output.wav",
    ] {
        assert_eq!(
            digest(&fixture.source.join("recordings").join(name)).unwrap(),
            digest(&fixture.archive_path().join("recordings").join(name)).unwrap()
        );
    }
    let imported_history_bytes = fs::read(&native_path).unwrap();
    let second = fixture.import(false, false).unwrap();
    assert_eq!(
        (
            second.history_added,
            second.meetings_added,
            second.templates_added
        ),
        (0, 0, 0)
    );
    assert_eq!(second.audio_files_copied, 0);
    assert_eq!(second.already_imported, 2);
    assert_eq!(fs::read(&native_path).unwrap(), imported_history_bytes);
    assert_eq!(
        crate::history::read_snapshot(&native_path).unwrap().len(),
        2
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&meeting).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(
                fixture
                    .archive_path()
                    .join("recordings/1730000100/output.ogg")
            )
            .unwrap()
            .permissions()
            .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn respects_history_off_and_no_audio_without_losing_metadata() {
    let fixture = Fixture::new();
    write_private(
        &fixture.support.join("settings.json"),
        br#"{"history_retention":"off"}"#,
    )
    .unwrap();
    fixture.record(
        "1730000000",
        &serde_json::json!({"result":"kept as metadata"}),
        "output.ogg",
    );
    write_private(
        &fixture.source.join("recordings/1730000000/output.WAV"),
        b"audio",
    )
    .unwrap();
    let report = fixture.import(false, true).unwrap();
    assert_eq!(report.history_added, 0);
    assert_eq!(report.history_excluded_by_retention, 1);
    assert_eq!(report.audio_files_copied, 0);
    assert!(
        fixture
            .archive_path()
            .join("recordings/1730000000/meta.json")
            .exists()
    );
    assert!(
        !fixture
            .archive_path()
            .join("recordings/1730000000/output.ogg")
            .exists()
    );
    assert!(!fixture.support.join("history.json").exists());
    assert!(
        !fixture
            .archive_path()
            .join("recordings/1730000000/output.WAV")
            .exists()
    );
}

#[test]
fn corrupt_hex_history_or_settings_stops_before_writing_any_import_data() {
    for filename in ["history.json", "settings.json"] {
        let fixture = Fixture::new();
        fixture.record(
            "1730000000",
            &serde_json::json!({"result":"hello"}),
            "output.wav",
        );
        let path = fixture.support.join(filename);
        fs::write(&path, b"damaged existing data").unwrap();
        assert!(fixture.import(false, false).is_err());
        assert!(!fixture.archive.exists());
        assert_eq!(fs::read(&path).unwrap(), b"damaged existing data");
    }
}

#[test]
fn changed_archive_files_are_never_overwritten_and_overlapping_paths_are_refused() {
    let fixture = Fixture::new();
    fixture.record(
        "1730000000",
        &serde_json::json!({"result":"hello"}),
        "output.wav",
    );
    fixture.import(false, false).unwrap();
    let archived = fixture
        .archive_path()
        .join("recordings/1730000000/output.wav");
    fs::write(&archived, b"user edited copy").unwrap();
    assert!(fixture.import(false, false).is_err());
    assert_eq!(fs::read(archived).unwrap(), b"user edited copy");
    assert!(
        import_into(
            &fixture.source,
            &fixture.source,
            &fixture.support,
            true,
            false
        )
        .is_err()
    );
}

#[cfg(unix)]
#[test]
fn ignores_source_symlinks_and_rejects_destination_symlinks() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new();
    let external = fixture.root.join("external");
    write_private(
        &external.join("1730000000/meta.json"),
        br#"{"result":"external"}"#,
    )
    .unwrap();
    symlink(
        external.join("1730000000"),
        fixture.source.join("recordings/1730000000"),
    )
    .unwrap();
    let report = fixture.import(false, false).unwrap();
    assert_eq!(report.recordings_seen, 0);
    assert!(!fixture.support.join("history.json").exists());
    fixture.record(
        "1730000100",
        &serde_json::json!({"result":"hello"}),
        "output.wav",
    );
    fs::create_dir_all(fixture.archive_path().join("recordings")).unwrap();
    symlink(
        &external,
        fixture.archive_path().join("recordings/1730000100"),
    )
    .unwrap();
    assert!(fixture.import(false, false).is_err());
    assert!(!external.join("meta.json").exists());
}

#[test]
fn timestamps_and_history_bounds_are_safe_for_untrusted_records() {
    assert_eq!(timestamp("1730000000", "").unwrap(), 1_730_000_000_000);
    assert_eq!(
        timestamp("0", "2024-10-27T03:33:20").unwrap(),
        1_730_000_000_000
    );
    assert!(timestamp("0", &"é".repeat(20)).is_err());
    let fixture = Fixture::new();
    let mut store = HistoryStore::open(
        fixture.support.join("history.json"),
        HistoryRetention::Forever,
        0,
    );
    let mut entry = history_entry(
        "1730000000",
        1,
        &Meta {
            result: "é".repeat(20_000),
            mode_name: "m".repeat(1000),
            ..Default::default()
        },
    );
    entry.source = Some("s".repeat(1000));
    assert_eq!(store.import([entry.clone(), entry], 0).unwrap(), 1);
    let saved = store.entries().next().unwrap();
    assert_eq!(saved.source.as_ref().unwrap().len(), 256);
    assert_eq!(saved.final_text, "é".repeat(8 * 1024));
    assert_eq!(saved.processing.as_ref().unwrap().profile.len(), 256);
    let turns = transcript_entries(&Meta {
        segments: vec![MetaSegment {
            text: "hello".into(),
            end: 1.0,
            speaker: Some(u64::MAX),
            ..Default::default()
        }],
        ..Default::default()
    });
    assert_eq!(turns[0].text, format!("[Speaker {}] hello", u64::MAX));
}

#[test]
fn separate_roots_keep_same_second_recordings_and_legacy_imports_stay_idempotent() {
    let first = Fixture::new();
    let second = Fixture::new();
    // An earlier importer wrote unscoped IDs and recorded its source here.
    write_private(
        &first.archive.join("import-report.json"),
        &serde_json::to_vec(&serde_json::json!({"version":1,"source":first.source})).unwrap(),
    )
    .unwrap();
    for (fixture, text) in [(&first, "first root"), (&second, "second root")] {
        fixture.record(
            "1730000000",
            &serde_json::json!({"result":text}),
            "output.wav",
        );
        fixture.record(
            "1730000001",
            &serde_json::json!({"kind":"meeting","result":text}),
            "output.wav",
        );
    }
    let legacy = first.import(false, false).unwrap();
    assert!(legacy.source_namespace.is_none());
    let other = import_into(&second.source, &first.archive, &first.support, false, false).unwrap();
    assert_eq!((other.history_added, other.meetings_added), (1, 1));
    assert_ne!(legacy.archive, other.archive);
    for (source, archive) in [
        (&first.source, &first.archive),
        (&second.source, &first.archive),
        (&second.source, &other.archive),
    ] {
        let again = import_into(source, archive, &first.support, false, false).unwrap();
        assert_eq!(
            (
                again.history_added,
                again.meetings_added,
                again.already_imported
            ),
            (0, 0, 2)
        );
    }
    let history = crate::history::read_snapshot(&first.support.join("history.json")).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        fs::read_dir(first.support.join("meetings"))
            .unwrap()
            .count(),
        2
    );
    assert!(
        history
            .iter()
            .any(|entry| entry.source.as_deref() == Some("superwhisper:1730000000"))
    );
}

#[test]
fn damaged_source_namespaces_are_not_treated_as_legacy_ids() {
    let fixture = Fixture::new();
    for bad in [
        Value::Null,
        serde_json::json!(42),
        serde_json::json!([]),
        serde_json::json!({}),
    ] {
        write_private(
            &fixture.archive.join("import-report.json"),
            &serde_json::to_vec(
                &serde_json::json!({"source":fixture.source,"source_namespace":bad}),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(fixture.import(false, false).is_err());
        assert!(!fixture.support.join("history.json").exists());
    }
}
