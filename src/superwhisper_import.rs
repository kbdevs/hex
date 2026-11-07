//! Explicit, offline import of Superwhisper's local recording folders.
//! The archive preserves originals; Hex's bounded history stores display copies.

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use clap::Args as ClapArgs;
use color_eyre::eyre::{Result, WrapErr, eyre};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::history::{
    HistoryEntry, HistoryKind, HistoryProcessing, HistoryRetention, HistoryStore,
};
use crate::meeting::{
    MeetingManifest, MeetingNotes, MeetingSource, MeetingStatus, TitleSource, TrackManifest,
    TranscriptEntry,
};

#[derive(ClapArgs, Debug)]
pub struct Args {
    /// Superwhisper data folder (default: ~/Documents/superwhisper).
    #[arg(long)]
    path: Option<PathBuf>,
    /// Original-file archive (default: ~/Documents/Hex Imports/Superwhisper).
    #[arg(long)]
    archive: Option<PathBuf>,
    /// Scan and report without writing. Hex can remain open.
    #[arg(long)]
    dry_run: bool,
    /// Omit audio from the archive.
    #[arg(long)]
    no_audio: bool,
    /// Print a versioned JSON report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Default, Serialize)]
struct Report {
    version: u32,
    source: PathBuf,
    archive: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_namespace: Option<String>,
    dry_run: bool,
    recordings_seen: usize,
    transcripts_found: usize,
    history_candidates: usize,
    history_added: usize,
    meetings_found: usize,
    meetings_added: usize,
    templates_found: usize,
    templates_added: usize,
    audio_files_copied: usize,
    files_copied: usize,
    history_excluded_by_retention: usize,
    already_imported: usize,
    warnings: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Meta {
    datetime: String,
    kind: String,
    title: String,
    duration: f64,
    mode_name: String,
    result: String,
    raw_result: String,
    llm_result: Option<String>,
    processing_time: u64,
    language_model_processing_time: u64,
    system_audio_enabled: bool,
    prompt_context: Value,
    segments: Vec<MetaSegment>,
    speakers: Vec<MetaSpeaker>,
}

impl Meta {
    fn final_text(&self) -> &str {
        self.llm_result
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .or_else(|| (!self.result.trim().is_empty()).then_some(self.result.as_str()))
            .unwrap_or(&self.raw_result)
            .trim()
    }

    fn raw_text(&self) -> &str {
        if self.raw_result.trim().is_empty() {
            self.result.trim()
        } else {
            self.raw_result.trim()
        }
    }

    fn is_meeting(&self) -> bool {
        self.kind == "meeting"
            || self.system_audio_enabled
            || self.mode_name.eq_ignore_ascii_case("meeting")
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetaSegment {
    text: String,
    start: f64,
    end: f64,
    speaker: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct MetaSpeaker {
    number: u64,
    name: String,
}

pub fn run(args: &Args) -> Result<()> {
    let documents =
        crate::app_paths::document_dir().ok_or_else(|| eyre!("Documents folder is unavailable"))?;
    let source = args
        .path
        .clone()
        .unwrap_or_else(|| documents.join("superwhisper"));
    let archive = args
        .archive
        .clone()
        .unwrap_or_else(|| documents.join("Hex Imports/Superwhisper"));
    let support = crate::app_paths::support_dir()?;
    // Take the same locks as the app before opening its writable history.
    // No force-quit: a recording must be stopped by its owner.
    let _locks = if args.dry_run {
        Vec::new()
    } else {
        ["local-api", "listener", "windows-listener", "service"]
            .into_iter()
            .map(crate::desktop::instance::acquire)
            .collect::<Result<Vec<_>>>()
            .wrap_err("Quit Hex before importing; use --dry-run to scan while it is open")?
    };
    let report = import_into(&source, &archive, &support, args.dry_run, args.no_audio)?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Superwhisper source: {}", report.source.display());
        println!("Original-file archive: {}", report.archive.display());
        println!("Recordings scanned: {}", report.recordings_seen);
        println!("History entries added: {}", report.history_added);
        println!("Meetings added: {}", report.meetings_added);
        println!("Templates added: {}", report.templates_added);
        println!("Audio files copied: {}", report.audio_files_copied);
        if report.dry_run {
            println!(
                "Dry run: {} history candidates; {} meetings; {} templates. Nothing written.",
                report.history_candidates, report.meetings_found, report.templates_found
            );
        }
        if report.history_excluded_by_retention > 0 {
            println!(
                "History entries outside the current retention: {} (originals stay in the archive)",
                report.history_excluded_by_retention
            );
        }
        for warning in &report.warnings {
            eprintln!("Warning: {warning}");
        }
    }
    Ok(())
}

fn import_into(
    source: &Path,
    archive: &Path,
    support: &Path,
    dry_run: bool,
    no_audio: bool,
) -> Result<Report> {
    let source = fs::canonicalize(source).wrap_err("could not open the Superwhisper folder")?;
    let archive = resolved(archive)?;
    let support = resolved(support)?;
    if source.starts_with(&archive)
        || archive.starts_with(&source)
        || source.starts_with(&support)
        || support.starts_with(&source)
        || support.starts_with(&archive)
        || archive.starts_with(&support)
    {
        return Err(eyre!(
            "source, archive, and Hex data must be separate folders"
        ));
    }
    let recordings = source.join("recordings");
    if !fs::symlink_metadata(&recordings)?.is_dir() {
        return Err(eyre!("recordings must be a real directory"));
    }
    let mut folders = fs::read_dir(&recordings)
        .wrap_err_with(|| format!("could not read {}", recordings.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    folders.sort_by_key(fs::DirEntry::file_name);

    // Validate existing history before any import writes. Never recover or
    // replace malformed user data as a side effect of migration.
    let history_path = support.join("history.json");
    if fs::symlink_metadata(&history_path).is_ok_and(|metadata| !metadata.is_file()) {
        return Err(eyre!("Hex history must be a regular file"));
    }
    let existing = crate::history::read_snapshot(&history_path)?;
    let mut known: HashSet<_> = existing
        .iter()
        .filter_map(|entry| entry.source.clone())
        .collect();
    let retention = read_retention(&support)?;
    let now = crate::history::now_ms();
    let cutoff = retention.max_age_ms().map(|age| now.saturating_sub(age));
    let (archive, namespace) = source_archive(&source, &archive)?;
    let mut report = Report {
        version: 1,
        source: source.clone(),
        archive: archive.clone(),
        source_namespace: namespace.clone(),
        dry_run,
        ..Report::default()
    };
    let mut history = Vec::new();

    for folder in folders {
        let name = folder.file_name();
        let Some(id) = name.to_str().filter(|id| !id.starts_with('.')) else {
            continue;
        };
        if !folder.file_type()?.is_dir() {
            report
                .warnings
                .push(format!("ignored non-directory recording {id}"));
            continue;
        }
        report.recordings_seen += 1;
        let path = folder.path();
        if !dry_run {
            copy_folder(
                &path,
                &archive.join("recordings").join(id),
                no_audio,
                &mut report,
            )?;
        }
        if id.parse::<u64>().is_err() {
            report
                .warnings
                .push(format!("{id}: expected an epoch-seconds folder name"));
            continue;
        }
        let meta: Meta = match read_json(&path.join("meta.json")) {
            Ok(meta) => meta,
            Err(error) => {
                report
                    .warnings
                    .push(format!("{id}: metadata unavailable ({error})"));
                continue;
            }
        };
        let timestamp_ms = match timestamp(id, &meta.datetime) {
            Ok(timestamp) => timestamp,
            Err(error) => {
                report.warnings.push(format!("{id}: {error}"));
                continue;
            }
        };
        if !meta.final_text().is_empty() {
            report.transcripts_found += 1;
        }
        let identity = namespace
            .as_ref()
            .map_or_else(|| id.to_owned(), |namespace| format!("{namespace}-{id}"));
        if meta.is_meeting() {
            report.meetings_found += 1;
            let target = support
                .join("meetings")
                .join(format!("superwhisper-{identity}"));
            if target.exists() {
                let manifest: MeetingManifest = read_json(&target.join("manifest.json"))?;
                if manifest.id != format!("superwhisper-{identity}") {
                    return Err(eyre!(
                        "existing meeting conflicts with Superwhisper recording {id}"
                    ));
                }
                report.already_imported += 1;
            } else if !dry_run {
                write_meeting(
                    id,
                    &identity,
                    timestamp_ms,
                    &meta,
                    &target,
                    &archive.join("recordings").join(id),
                )?;
                report.meetings_added += 1;
            }
        } else if !meta.final_text().is_empty() {
            let entry = history_entry(&identity, timestamp_ms, &meta);
            if !known.insert(entry.source.clone().expect("imported history has a source")) {
                report.already_imported += 1;
            } else if retention.is_off() || cutoff.is_some_and(|cutoff| timestamp_ms < cutoff) {
                report.history_excluded_by_retention += 1;
            } else {
                history.push(entry);
                report.history_candidates += 1;
            }
        }
    }

    // Preserve local configuration as files. Do not import licenses, model
    // service accounts, scripts as executable code, or account credentials.
    for name in ["modes", "meeting-templates"] {
        let directory = source.join(name);
        if !directory.exists() {
            continue;
        }
        if !fs::symlink_metadata(&directory)?.is_dir() {
            return Err(eyre!(
                "refusing symbolic-link configuration folder {}",
                directory.display()
            ));
        }
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry.path().extension().and_then(|s| s.to_str()) != Some("json")
            {
                continue;
            }
            if name == "meeting-templates" {
                crate::meeting_templates::read(&entry.path())?;
                report.templates_found += 1;
                if !dry_run
                    && copy_file(&entry.path(), &support.join(name).join(entry.file_name()))?
                {
                    report.templates_added += 1;
                }
            }
            if !dry_run {
                copy_file(&entry.path(), &archive.join(name).join(entry.file_name()))?;
            }
        }
    }
    let settings = source.join("settings.json");
    if settings.is_file() && !dry_run {
        copy_file(&settings, &archive.join("settings.json"))?;
    }
    if !dry_run {
        if history_path.exists() {
            let backup = archive
                .join("hex-before-import")
                .join(format!("history-{}.json", digest(&history_path)?));
            copy_file(&history_path, &backup)?;
        }
        if !history.is_empty() {
            let mut store = HistoryStore::open(history_path, retention, now);
            report.history_added = store.import(history, now)?;
        }
        write_private(
            &archive.join("import-report.json"),
            &serde_json::to_vec_pretty(&report)?,
        )?;
        write_private(&archive.join("README.txt"), b"Superwhisper import\n\nrecordings/ keeps original metadata and audio. Empty and damaged records are also kept here.\nHex history contains bounded display copies of dictations. Meetings are in Hex's Meetings pane.\nNamed speakers are kept in transcript text; Hex's generic You/Them headers do not identify imported speakers.\nMeeting templates are available with: hex meetings templates list\nOriginal configuration is archived as JSON. Scripts are not run. Account credentials are not imported.\nhex-before-import/ holds history snapshots from before each import. import-report.json records the latest run.\nRepeated imports skip existing history and meetings. Changed archive files cause an error instead of being overwritten.\n")?;
    }
    Ok(report)
}

/// Existing reports bind legacy IDs to their original root. New roots get
/// separate IDs and archive folders, even when their timestamps overlap.
fn source_archive(source: &Path, archive: &Path) -> Result<(PathBuf, Option<String>)> {
    let namespace: String = Sha256::digest(source.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let report = archive.join("import-report.json");
    if report.exists() {
        let report: Value = read_json(&report)?;
        if report.get("source") == Some(&serde_json::to_value(source)?) {
            let saved = match report.get("source_namespace") {
                None => None,
                Some(Value::String(saved)) => Some(saved.as_str()),
                Some(_) => return Err(eyre!("archive source identity must be text")),
            };
            if saved.is_some_and(|saved| saved != namespace) {
                return Err(eyre!("archive source identity does not match its report"));
            }
            return Ok((archive.to_owned(), saved.map(str::to_owned)));
        }
    }
    Ok((archive.join("sources").join(&namespace), Some(namespace)))
}

fn history_entry(id: &str, timestamp_ms: u64, meta: &Meta) -> HistoryEntry {
    let application = meta
        .prompt_context
        .pointer("/applicationContext/name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    HistoryEntry {
        id: 0,
        timestamp_ms,
        kind: HistoryKind::Dictation,
        raw_text: meta.raw_text().into(),
        final_text: meta.final_text().into(),
        application,
        processing: (!meta.mode_name.trim().is_empty()).then(|| HistoryProcessing {
            profile: meta.mode_name.trim().into(),
            latency_ms: meta.language_model_processing_time,
            fallback: None,
        }),
        audio_ms: meta.duration.max(0.0) as u64,
        inference_ms: meta.processing_time,
        total_ms: meta
            .processing_time
            .saturating_add(meta.language_model_processing_time),
        source: Some(format!("superwhisper:{id}")),
        recording_bytes: 0,
        model: None,
        language: None,
        text_ms: None,
        versions: Vec::new(),
    }
}

fn timestamp(id: &str, datetime: &str) -> Result<u64> {
    if !datetime.is_empty() {
        // Superwhisper's older ISO strings omit the zone but represent UTC.
        let datetime = if datetime.len() >= 19
            && !datetime
                .get(19..)
                .unwrap_or("")
                .contains(['Z', 'z', '+', '-'])
        {
            format!("{datetime}Z")
        } else {
            datetime.to_owned()
        };
        return crate::meeting::query::parse_time(&datetime, 0, false);
    }
    id.parse::<u64>()
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000))
        .ok_or_else(|| eyre!("no valid recording timestamp"))
}

fn write_meeting(
    id: &str,
    identity: &str,
    timestamp_ms: u64,
    meta: &Meta,
    target: &Path,
    archive: &Path,
) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| eyre!("meeting folder has no parent"))?;
    private_dir(parent)?;
    let staging = staging_path(target);
    private_dir(&staging)?;
    let result = (|| -> Result<()> {
        let entries = transcript_entries(meta);
        let duration_ms = (meta.duration.max(0.0) as u64)
            .max(entries.iter().map(|entry| entry.end_ms).max().unwrap_or(0));
        let title = if meta.title.trim().is_empty() {
            format!("Superwhisper meeting {id}")
        } else {
            meta.title.trim().to_owned()
        };
        let notes = meta
            .llm_result
            .as_deref()
            .filter(|text| !text.trim().is_empty())
            .map(|text| MeetingNotes {
                title: title.clone(),
                summary: text.trim().to_owned(),
                imported_from: Some("Superwhisper".into()),
                ..Default::default()
            });
        let manifest = MeetingManifest {
            schema_version: 1,
            id: format!("superwhisper-{identity}"),
            title: title.clone(),
            status: MeetingStatus::Complete,
            created_at_ms: timestamp_ms,
            ended_at_ms: Some(timestamp_ms.saturating_add(duration_ms)),
            duration_ms: Some(duration_ms),
            system: TrackManifest::default(),
            microphone: TrackManifest::default(),
            live_transcription_error: None,
            error: None,
            app: Some("Superwhisper".into()),
            export_path: None,
            export_error: None,
            echo_suppressed_utterances: 0,
            provisional_title: None,
            notes,
            notes_error: None,
            calendar: None,
            title_source: Some(TitleSource::User),
            chapters: Vec::new(),
            chapters_refined: false,
            retention_applies: false,
            export_sha256: None,
            imported: true,
        };
        write_private(
            &staging.join("manifest.json"),
            &serde_json::to_vec_pretty(&manifest)?,
        )?;
        let mut ndjson = Vec::new();
        for entry in &entries {
            serde_json::to_writer(&mut ndjson, entry)?;
            ndjson.push(b'\n');
        }
        write_private(&staging.join("transcript.ndjson"), &ndjson)?;
        let notes_text = manifest
            .notes
            .as_ref()
            .map(crate::meeting::notes_markdown)
            .unwrap_or_default();
        let markdown = format!(
            "# {title}\n\n{notes_text}\n\n## Transcript\n\n{}\n",
            crate::meeting::transcript_markdown(&entries)
        );
        write_private(&staging.join("transcript.md"), markdown.as_bytes())?;
        write_private(
            &staging.join("superwhisper-source.json"),
            &serde_json::to_vec_pretty(&serde_json::json!({
                "source": "superwhisper", "recording": id, "original_files": archive,
                "speaker_labels": "Names are embedded in transcript text; no You/Them identity is inferred."
            }))?,
        )?;
        fs::rename(&staging, target)?;
        crate::durable::sync_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(&staging);
    }
    result
}

fn transcript_entries(meta: &Meta) -> Vec<TranscriptEntry> {
    let mut turns: Vec<(Option<u64>, TranscriptEntry)> = Vec::new();
    let mut segments = meta
        .segments
        .iter()
        .filter(|segment| !segment.text.trim().is_empty())
        .collect::<Vec<_>>();
    segments.sort_by(|left, right| left.start.total_cmp(&right.start));
    for segment in segments {
        let start_ms = (segment.start.max(0.0) * 1_000.0) as u64;
        let end_ms = ((segment.end.max(segment.start).max(0.0) * 1_000.0) as u64)
            .max(start_ms.saturating_add(1));
        if let Some((speaker, turn)) = turns.last_mut()
            && *speaker == segment.speaker
            && start_ms.saturating_sub(turn.end_ms) <= 3_000
        {
            turn.text.push(' ');
            turn.text.push_str(segment.text.trim());
            turn.end_ms = turn.end_ms.max(end_ms);
            continue;
        }
        let speaker = segment.speaker.and_then(|number| {
            meta.speakers
                .iter()
                .find(|speaker| speaker.number == number)
        });
        let label = speaker
            .map(|speaker| speaker.name.trim())
            .filter(|name| !name.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                segment
                    .speaker
                    .map(|number| format!("Speaker {}", number.saturating_add(1)))
            });
        let text = label.map_or_else(
            || segment.text.trim().to_owned(),
            |label| format!("[{label}] {}", segment.text.trim()),
        );
        turns.push((
            segment.speaker,
            TranscriptEntry {
                source: MeetingSource::System,
                start_ms,
                end_ms,
                text,
            },
        ));
    }
    if turns.is_empty() && !meta.raw_text().is_empty() {
        turns.push((
            None,
            TranscriptEntry {
                source: MeetingSource::System,
                start_ms: 0,
                end_ms: (meta.duration.max(1.0) as u64),
                text: meta.raw_text().into(),
            },
        ));
    }
    turns.into_iter().map(|(_, entry)| entry).collect()
}

fn copy_folder(
    source: &Path,
    destination: &Path,
    no_audio: bool,
    report: &mut Report,
) -> Result<()> {
    private_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if !file_type.is_file() {
            report.warnings.push(format!(
                "ignored non-regular recording file {}",
                path.display()
            ));
            continue;
        }
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        let audio = matches!(
            path.extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("wav" | "ogg" | "m4a" | "mp3" | "aac" | "caf" | "flac" | "opus")
        );
        if no_audio && audio {
            continue;
        }
        if copy_file(&path, &destination.join(entry.file_name()))? {
            report.files_copied += 1;
            report.audio_files_copied += usize::from(audio);
        }
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path) -> Result<bool> {
    if !fs::symlink_metadata(source)?.is_file() {
        return Err(eyre!(
            "refusing non-regular source file {}",
            source.display()
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(destination) {
        if metadata.is_file() && digest(source)? == digest(destination)? {
            return Ok(false);
        }
        return Err(eyre!(
            "destination file differs; nothing was overwritten: {}",
            destination.display()
        ));
    }
    let parent = destination
        .parent()
        .ok_or_else(|| eyre!("destination has no parent"))?;
    private_dir(parent)?;
    let temporary = staging_path(destination);
    private_dir(&temporary)?;
    let copied = (|| -> Result<()> {
        let staged = temporary.join("data");
        fs::copy(source, &staged)?;
        let output = OpenOptions::new().write(true).open(&staged)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            output.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        output.sync_all()?;
        fs::rename(&staged, destination)?;
        fs::remove_dir(&temporary)?;
        crate::durable::sync_directory(parent)?;
        Ok(())
    })();
    if copied.is_err() {
        let _ = fs::remove_dir_all(&temporary);
    }
    copied?;
    Ok(true)
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| eyre!("destination has no parent"))?;
    private_dir(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.is_file()) {
        return Err(eyre!("refusing non-regular output {}", path.display()));
    }
    let temporary = staging_path(path);
    let written = (|| -> Result<()> {
        let mut file = private_file(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        crate::durable::sync_directory(parent)?;
        Ok(())
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

fn private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn private_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        private_dir(parent)?;
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => return Ok(()),
        Ok(_) => {
            return Err(eyre!(
                "refusing non-directory destination {}",
                path.display()
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    fs::create_dir(path)?;
    Ok(())
}

fn staging_path(path: &Path) -> PathBuf {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(
        ".{}.{}.{sequence}.tmp",
        path.file_name().unwrap_or_default().to_string_lossy(),
        std::process::id()
    ))
}

fn resolved(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return Ok(fs::canonicalize(path)?);
    }
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute
        .parent()
        .ok_or_else(|| eyre!("invalid destination {}", path.display()))?;
    let name = absolute
        .file_name()
        .ok_or_else(|| eyre!("use a destination path without '..'"))?;
    Ok(resolved(parent)?.join(name))
}

fn read_retention(root: &Path) -> Result<HistoryRetention> {
    #[derive(Default, Deserialize)]
    #[serde(default)]
    struct Settings {
        history_retention: HistoryRetention,
    }
    let path = root.join("settings.json");
    if !path.exists() {
        return Ok(HistoryRetention::default());
    }
    let settings: Settings =
        read_json(&path).wrap_err("Hex settings are invalid; import stopped")?;
    Ok(settings.history_retention)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T> {
    if !fs::symlink_metadata(path)?.is_file() {
        return Err(eyre!("not a regular file: {}", path.display()));
    }
    const LIMIT: u64 = 16 * 1024 * 1024;
    let mut bytes = Vec::new();
    File::open(path)?.take(LIMIT + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > LIMIT {
        return Err(eyre!("JSON exceeds 16 MiB: {}", path.display()));
    }
    serde_json::from_slice(&bytes).wrap_err_with(|| format!("could not decode {}", path.display()))
}

fn digest(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

#[cfg(test)]
mod tests;
