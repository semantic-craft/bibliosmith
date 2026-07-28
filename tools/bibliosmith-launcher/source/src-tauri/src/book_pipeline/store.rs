//! Durable state: the on-disk book-pipeline-state file and the locks that guard it. Owns the advisory file lock, the optimistic revision guard, the tempfile+rename atomic write, the per-run execution lease, and the recovery pass that reclaims stages orphaned when a previous run died holding one.

use super::*;

pub(crate) trait BookPipelineStateStore: Send + Sync {
    fn load(&self) -> Result<BookPipelineState, String>;
    fn save(&self, state: &BookPipelineState) -> Result<(), String>;
    fn job_output_dir(&self, job_id: &str) -> PathBuf;
    fn execution_owner(&self) -> Result<&str, String>;
}

#[derive(Debug)]
pub(crate) struct BookPipelineStore {
    pub(crate) state_path: PathBuf,
    output_root: PathBuf,
    execution_owner: String,
    execution_lease_root: PathBuf,
    execution_lease: Mutex<Option<Arc<File>>>,
}

pub(crate) struct BookPipelineStoreLock {
    path: PathBuf,
}

impl Drop for BookPipelineStoreLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

impl BookPipelineStore {
    pub(crate) fn default() -> Result<Self, String> {
        let state_dir = default_state_dir()?;
        Self::new(
            state_dir.join("jobs.json"),
            default_output_root()?,
            new_execution_owner(),
        )
    }

    #[cfg(test)]
    pub(crate) fn for_test(root: &Path) -> Self {
        Self::new(
            root.join("state").join("jobs.json"),
            root.join("output"),
            new_execution_owner(),
        )
        .expect("test Book Pipeline store should acquire its execution lease")
    }

    #[cfg(test)]
    pub(crate) fn for_test_with_owner(root: &Path, execution_owner: &str) -> Self {
        Self::new(
            root.join("state").join("jobs.json"),
            root.join("output"),
            execution_owner.into(),
        )
        .expect("test Book Pipeline store should acquire its execution lease")
    }

    fn new(
        state_path: PathBuf,
        output_root: PathBuf,
        execution_owner: String,
    ) -> Result<Self, String> {
        let state_root = state_path
            .parent()
            .ok_or_else(|| "Book Pipeline state path has no parent directory.".to_string())?;
        let execution_lease_root = state_root.join("execution-leases");
        Ok(Self {
            state_path,
            output_root,
            execution_owner,
            execution_lease_root,
            execution_lease: Mutex::new(None),
        })
    }

    fn read_state_unlocked(&self) -> Result<BookPipelineState, String> {
        if !self.state_path.exists() {
            return Ok(BookPipelineState::default());
        }
        let text = fs::read_to_string(&self.state_path).map_err(|err| err.to_string())?;
        serde_json::from_str(&text).map_err(|err| err.to_string())
    }

    fn acquire_lock(&self) -> Result<BookPipelineStoreLock, String> {
        let Some(parent) = self.state_path.parent() else {
            return Err("Book Pipeline state path has no parent directory.".into());
        };
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let lock_path = self.state_path.with_extension("json.lock");
        for _ in 0..200 {
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(mut lock_file) => {
                    let guard = BookPipelineStoreLock {
                        path: lock_path.clone(),
                    };
                    writeln!(lock_file, "{}", std::process::id()).map_err(|err| err.to_string())?;
                    return Ok(guard);
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock_path)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| modified.elapsed().ok())
                        .is_some_and(|age| age > Duration::from_secs(30));
                    if stale {
                        let _ = fs::remove_file(&lock_path);
                    } else {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                Err(error) => return Err(error.to_string()),
            }
        }
        Err("Book Pipeline state store is busy.".into())
    }

    pub(crate) fn write_state_unlocked(&self, state: &BookPipelineState) -> Result<(), String> {
        let parent = self
            .state_path
            .parent()
            .ok_or_else(|| "Book Pipeline state path has no parent directory.".to_string())?;
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        let text = serde_json::to_string_pretty(state).map_err(|err| err.to_string())?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
        temporary
            .write_all(text.as_bytes())
            .map_err(|err| err.to_string())?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|err| err.to_string())?;
        temporary
            .persist(&self.state_path)
            .map_err(|err| err.error.to_string())?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|err| err.to_string())?;
        Ok(())
    }
}

impl BookPipelineStateStore for BookPipelineStore {
    fn load(&self) -> Result<BookPipelineState, String> {
        let mut state = self.read_state_unlocked()?;
        let needs_migration = state_needs_upgrade(&state);
        let needs_recovery = has_interrupted_running_stage(&state, &self.execution_lease_root);
        if needs_migration || needs_recovery {
            let _lock = self.acquire_lock()?;
            state = self.read_state_unlocked()?;
            let mut changed = false;
            let migrated = state_needs_upgrade(&state);
            if migrated {
                migrate_legacy_state(&mut state);
                mark_migrated_interrupted_stages(&mut state);
                changed = true;
            }
            if !migrated && recover_interrupted_stages(&mut state, &self.execution_lease_root) {
                changed = true;
            }
            if changed {
                state.revision = state.revision.saturating_add(1);
                derive_state(&mut state);
                validate_state(&state)?;
                self.write_state_unlocked(&state)?;
            }
        }
        derive_state(&mut state);
        validate_state(&state)?;
        Ok(state)
    }

    fn save(&self, state: &BookPipelineState) -> Result<(), String> {
        let _lock = self.acquire_lock()?;
        let mut previous = self.read_state_unlocked()?;
        if state_needs_upgrade(&previous) {
            migrate_legacy_state(&mut previous);
            mark_migrated_interrupted_stages(&mut previous);
        } else {
            derive_state(&mut previous);
        }
        if previous.revision != state.revision {
            return Err(format!(
                "Book Pipeline state changed concurrently: expected revision {}, found {}.",
                state.revision, previous.revision
            ));
        }

        let mut next = state.clone();
        next.schema_version = STATE_SCHEMA_VERSION.into();
        let mut migrated = false;
        for job in &mut next.jobs {
            if job_needs_upgrade(job) {
                migrate_legacy_job(job);
                migrated = true;
            }
        }
        if migrated {
            mark_migrated_interrupted_stages(&mut next);
        }
        derive_state(&mut next);
        validate_state(&next)?;
        validate_state_transitions(&previous, &next)?;
        next.revision = state.revision.saturating_add(1);
        self.write_state_unlocked(&next)
    }

    fn job_output_dir(&self, job_id: &str) -> PathBuf {
        self.output_root.join(job_id)
    }

    fn execution_owner(&self) -> Result<&str, String> {
        let mut lease = self
            .execution_lease
            .lock()
            .map_err(|_| "Book Pipeline execution lease is poisoned.".to_string())?;
        if lease.is_none() {
            *lease = Some(acquire_execution_lease(
                &self.execution_lease_root,
                &self.execution_owner,
            )?);
        }
        Ok(&self.execution_owner)
    }
}

impl Drop for BookPipelineStore {
    fn drop(&mut self) {
        let Ok(lease) = self.execution_lease.get_mut() else {
            return;
        };
        let Some(lease) = lease.take() else {
            return;
        };
        if Arc::strong_count(&lease) == 1 {
            let path = execution_lease_path(&self.execution_lease_root, &self.execution_owner);
            let _ = FileExt::unlock(lease.as_ref());
            drop(lease);
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn state_needs_upgrade(state: &BookPipelineState) -> bool {
    state.schema_version != STATE_SCHEMA_VERSION || state.jobs.iter().any(job_needs_upgrade)
}

pub(crate) fn job_needs_upgrade(job: &BookPipelineJob) -> bool {
    job.schema_version != JOB_SCHEMA_VERSION
        || job.translation_mode.is_empty()
        || (job.children.is_empty() && (!job.route.is_empty() || !job.collection_items.is_empty()))
        || job
            .children
            .iter()
            .any(|child| child.stages.iter().all(|stage| stage.stage_id != "index"))
        || job
            .stages
            .iter()
            .chain(job.children.iter().flat_map(|child| child.stages.iter()))
            .any(|stage| stage.contract_version.is_empty())
}

pub(crate) fn new_execution_owner() -> String {
    static OWNER_SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = OWNER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("execution-{}-{started}-{sequence}", std::process::id())
}

pub(crate) fn execution_lease_path(root: &Path, execution_owner: &str) -> PathBuf {
    let digest = Sha256::digest(execution_owner.as_bytes());
    root.join(format!("{digest:x}.lock"))
}

pub(crate) fn acquire_execution_lease(
    root: &Path,
    execution_owner: &str,
) -> Result<Arc<File>, String> {
    static LEASES: OnceLock<Mutex<BTreeMap<PathBuf, Weak<File>>>> = OnceLock::new();
    fs::create_dir_all(root).map_err(|error| error.to_string())?;
    let path = execution_lease_path(root, execution_owner);
    let mut leases = LEASES
        .get_or_init(|| Mutex::new(BTreeMap::new()))
        .lock()
        .map_err(|_| "Book Pipeline execution lease registry is poisoned.".to_string())?;
    leases.retain(|_, lease| lease.strong_count() > 0);
    if let Some(lease) = leases.get(&path).and_then(Weak::upgrade) {
        return Ok(lease);
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|error| error.to_string())?;
    match FileExt::try_lock(&file) {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            return Err(format!(
                "Book Pipeline execution owner {execution_owner} is already active."
            ));
        }
        Err(TryLockError::Error(error)) => return Err(error.to_string()),
    }
    let lease = Arc::new(file);
    leases.insert(path, Arc::downgrade(&lease));
    Ok(lease)
}

pub(crate) fn execution_lease_is_active(root: &Path, execution_owner: &str) -> bool {
    let path = execution_lease_path(root, execution_owner);
    let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
        return false;
    };
    match FileExt::try_lock(&file) {
        Ok(()) => {
            let _ = FileExt::unlock(&file);
            drop(file);
            let _ = fs::remove_file(path);
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => true,
    }
}

pub(crate) fn has_interrupted_running_stage(
    state: &BookPipelineState,
    execution_lease_root: &Path,
) -> bool {
    state.jobs.iter().any(|job| {
        job.stages
            .iter()
            .chain(job.children.iter().flat_map(|child| child.stages.iter()))
            .any(|stage| running_stage_is_interrupted(stage, execution_lease_root))
    })
}

pub(crate) fn mark_migrated_interrupted_stages(state: &mut BookPipelineState) {
    for job in &mut state.jobs {
        for stage in job.stages.iter_mut().chain(
            job.children
                .iter_mut()
                .flat_map(|child| child.stages.iter_mut()),
        ) {
            if stage.status == STATUS_RUNNING && stage.execution_owner.is_none() {
                stage.execution_owner = Some(MIGRATED_INTERRUPTED_OWNER.into());
            }
        }
    }
}

pub(crate) fn running_stage_is_interrupted(
    stage: &BookPipelineStage,
    execution_lease_root: &Path,
) -> bool {
    if stage.status != STATUS_RUNNING {
        return false;
    }
    match stage.execution_owner.as_deref() {
        Some(owner) => !execution_lease_is_active(execution_lease_root, owner),
        None => true,
    }
}

pub(crate) fn recover_interrupted_stages(
    state: &mut BookPipelineState,
    execution_lease_root: &Path,
) -> bool {
    let mut recovered = false;
    let finished_at = now_label();
    for job in &mut state.jobs {
        for stage in job.stages.iter_mut().chain(
            job.children
                .iter_mut()
                .flat_map(|child| child.stages.iter_mut()),
        ) {
            if running_stage_is_interrupted(stage, execution_lease_root) {
                stage.status = STATUS_FAILED.into();
                stage.error = Some("Stage interrupted by launcher restart; retry is safe.".into());
                stage.finished_at = Some(finished_at.clone());
                stage.execution_owner = None;
                recovered = true;
            }
        }
    }
    recovered
}
