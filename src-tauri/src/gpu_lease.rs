//! One OS lock shared with standalone Studio workers. Owners inside this process
//! can cooperate (a benchmark stops/restarts the server) without unlocking early.
use std::{
    collections::HashSet,
    fs::File,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

#[derive(Default)]
struct Lease {
    file: Option<File>,
    owners: HashSet<&'static str>,
}
static STATE: OnceLock<(PathBuf, Mutex<Lease>)> = OnceLock::new();

pub fn init(data: &std::path::Path) {
    let _ = STATE.set((data.join("gpu.lock"), Mutex::new(Lease::default())));
}

pub fn acquire(owner: &'static str) -> Result<(), String> {
    let Some((path, state)) = STATE.get() else {
        return Ok(());
    };
    let mut state = state
        .lock()
        .map_err(|_| "Não foi possível reservar a GPU")?;
    if state.file.is_none() {
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .map_err(|e| e.to_string())?;
        file.try_lock().map_err(|_| "Outro OpenWeights ou Studio está usando a GPU. Pare o trabalho nele antes de continuar.".to_string())?;
        state.file = Some(file);
    }
    state.owners.insert(owner);
    Ok(())
}

pub fn release(owner: &'static str) {
    if let Some((_, state)) = STATE.get()
        && let Ok(mut state) = state.lock()
    {
        state.owners.remove(owner);
        if state.owners.is_empty() {
            state.file = None;
        }
    }
}

pub struct Guard(&'static str);
impl Guard {
    pub fn new(owner: &'static str) -> Result<Self, String> {
        acquire(owner)?;
        Ok(Self(owner))
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        release(self.0);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn kernel_lock_excludes_another_owner_until_drop() {
        let path = std::env::temp_dir().join(format!("ow-gpu-lock-test-{}", std::process::id()));
        let first = std::fs::File::options()
            .write(true)
            .read(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .unwrap();
        let second = std::fs::File::options()
            .write(true)
            .read(true)
            .open(&path)
            .unwrap();
        first.try_lock().unwrap();
        assert!(second.try_lock().is_err());
        drop(first);
        second.try_lock().unwrap();
        drop(second);
        std::fs::remove_file(path).unwrap();
    }
}
