//! Ячейка значения, которое строится при первом обращении

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use once_cell::race::OnceBox;

/// Как часто поток проверяет, построено ли значение, пока его строит другой поток
const WAIT: Duration = Duration::from_micros(100);

/// Значение, которое при первом обращении строит один поток процесса, а остальные ждут его.
/// Ячейка - указатель без состояния «занято», в отличие от `OnceLock`: fork, заставший сборку,
/// не оставляет дочерний процесс ждать поток, которого в нем нет, - ребенок строит значение сам
pub(crate) struct BuildOnce<T> {
    value: OnceBox<T>,
    /// Процесс, поток которого строит значение; 0 - никто
    builder: AtomicU32,
}

impl<T> BuildOnce<T> {
    pub(crate) fn new() -> Self {
        BuildOnce {
            value: OnceBox::new(),
            builder: AtomicU32::new(0),
        }
    }

    pub(crate) fn get_or_build(&self, build: impl FnOnce() -> T) -> &T {
        if let Some(value) = self.value.get() {
            return value;
        }
        let process = std::process::id();
        loop {
            if let Some(value) = self.value.get() {
                return value;
            }
            // Сборку другого процесса - родителя до fork - его поток здесь не закончит
            let builder = self.builder.load(Ordering::Acquire);
            if builder != process
                && self
                    .builder
                    .compare_exchange(builder, process, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
            {
                break;
            }
            std::thread::sleep(WAIT);
        }
        let _release = Release(&self.builder);
        self.value.get_or_init(|| Box::new(build()))
    }
}

/// Снимает отметку сборщика и при панике сборки: иначе ждущие потоки ждали бы вечно
struct Release<'a>(&'a AtomicU32);

impl Drop for Release<'_> {
    fn drop(&mut self) {
        self.0.store(0, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Barrier;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn one_build_for_threads() {
        let cell = BuildOnce::new();
        let builds = AtomicUsize::new(0);
        let barrier = Barrier::new(16);
        std::thread::scope(|scope| {
            for _ in 0..16 {
                scope.spawn(|| {
                    barrier.wait();
                    let value = cell.get_or_build(|| {
                        builds.fetch_add(1, Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(20));
                        42
                    });
                    assert_eq!(*value, 42);
                });
            }
        });
        assert_eq!(builds.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn builder_of_other_process() {
        // Так ячейку видит ребенок fork, заставшего сборку в родителе
        let cell = BuildOnce::new();
        cell.builder
            .store(std::process::id().wrapping_add(1), Ordering::Relaxed);
        assert_eq!(*cell.get_or_build(|| 7), 7);
    }

    #[test]
    fn panic_in_build() {
        let cell = BuildOnce::new();
        let built = std::panic::catch_unwind(|| cell.get_or_build(|| panic!("сборка")));
        assert!(built.is_err());
        assert_eq!(*cell.get_or_build(|| 3), 3);
    }
}
