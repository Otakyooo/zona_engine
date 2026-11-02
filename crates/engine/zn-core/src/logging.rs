//! Логи сервиса: stderr плюс файл с ротацией по размеру.
//!
//! `env_logger` умеет писать только в один приёмник, поэтому консоль и файл
//! объединены через `Tee`: поведение в терминале не меняется, а лог переживает
//! закрытие окна. Ротация ручная — тянуть `tracing-appender` ради одного файла
//! дороже, чем 60 строк здесь.

use std::fs::{File, OpenOptions, create_dir_all};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Размер, после которого файл уходит в `.log.1`. По умолчанию 5 МиБ.
pub const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;

/// Файл, который при переполнении переименовывается в `<имя>.log.1`.
pub struct RotatingFile {
    path: PathBuf,
    file: File,
    written: u64,
    max_bytes: u64,
}

impl RotatingFile {
    pub fn open(path: impl AsRef<Path>, max_bytes: u64) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(dir) = path.parent()
            && !dir.as_os_str().is_empty()
        {
            create_dir_all(dir)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let written = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            written,
            max_bytes: max_bytes.max(1),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Сколько байт записано в текущий файл.
    pub fn written(&self) -> u64 {
        self.written
    }

    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        let backup = self.path.with_extension("log.1");
        // Windows: `rename` поверх существующего файла не удаётся, поэтому
        // прошлый бэкап удаляется заранее.
        if backup.exists() {
            std::fs::remove_file(&backup)?;
        }
        std::fs::rename(&self.path, &backup)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.written = 0;
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        // Пустая запись не должна крутить файл на ровном месте.
        if !buf.is_empty() && self.written > 0 && self.written + buf.len() as u64 > self.max_bytes {
            self.rotate()?;
        }
        let n = self.file.write(buf)?;
        self.written += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Дублирует вывод в два приёмника.
///
/// Ошибка файла не должна глушить консоль: диагностика в терминале важнее
/// полноты файла, иначе сломанный диск прячет причину поломки.
pub struct Tee<A: Write, B: Write> {
    pub primary: A,
    pub secondary: B,
}

impl<A: Write, B: Write> Write for Tee<A, B> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.primary.write(buf)?;
        let _ = self.secondary.write_all(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        let _ = self.primary.flush();
        self.secondary.flush()
    }
}

/// Инициализирует логи сервиса: stderr + `logs/<service>.log`.
///
/// Каталог задаётся `ZONA_LOG_DIR`, лимит файла — `ZONA_LOG_MAX_BYTES`.
/// Повторный вызов не паникует: второй логгер просто не встаёт.
pub fn init(service: &str) {
    let dir = std::env::var("ZONA_LOG_DIR").unwrap_or_else(|_| "logs".to_string());
    let max_bytes = std::env::var("ZONA_LOG_MAX_BYTES")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_BYTES);
    let path = Path::new(&dir).join(format!("{service}.log"));

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    match RotatingFile::open(&path, max_bytes) {
        Ok(file) => {
            builder.target(env_logger::Target::Pipe(Box::new(Tee {
                primary: io::stderr(),
                secondary: file,
            })));
        }
        Err(e) => eprintln!("не удалось открыть лог {}: {e}", path.display()),
    }
    // `try_init` вернёт ошибку, если логгер уже стоит — это не повод падать.
    let _ = builder.try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    /// Уникальный каталог в temp: тесты идут параллельно и не должны видеть
    /// файлы друг друга.
    fn temp_dir(name: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("zona-log-{}-{}-{n}", std::process::id(), name))
    }

    fn read(path: &Path) -> String {
        let mut s = String::new();
        File::open(path).unwrap().read_to_string(&mut s).unwrap();
        s
    }

    #[test]
    fn rotates_only_when_over_limit() {
        let dir = temp_dir("rotate");
        let path = dir.join("test.log");
        {
            let mut f = RotatingFile::open(&path, 8).unwrap();
            f.write_all(b"aaaa").unwrap();
            f.write_all(b"bbbb").unwrap();
            assert_eq!(f.written(), 8, "ровно на лимите ротации ещё нет");
            f.write_all(b"cccc").unwrap();
        }
        assert_eq!(read(&path), "cccc");
        assert_eq!(read(&path.with_extension("log.1")), "aaaabbbb");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn appends_below_limit() {
        let dir = temp_dir("append");
        let path = dir.join("test.log");
        {
            let mut f = RotatingFile::open(&path, 100).unwrap();
            f.write_all(b"aa").unwrap();
            f.write_all(b"bb").unwrap();
        }
        assert_eq!(read(&path), "aabb");
        assert!(!path.with_extension("log.1").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tee_writes_to_both_sinks() {
        let mut console: Vec<u8> = Vec::new();
        let mut file: Vec<u8> = Vec::new();
        {
            let mut tee = Tee {
                primary: &mut console,
                secondary: &mut file,
            };
            tee.write_all(b"log line").unwrap();
        }
        assert_eq!(console, b"log line");
        assert_eq!(file, b"log line");
    }
}
