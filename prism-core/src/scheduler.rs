//! PrismOS cooperative scheduler.
//!
//! ROLE:
//! Demonstrates multitasking WITHOUT interrupts, preemption, or context
//! switching. Each task is a small step function called in round-robin
//! order. This proves the scheduling DATA STRUCTURE before we add the
//! dangerous parts (timer interrupts, register saving, privilege levels).
//!
//! WHY COOPERATIVE FIRST:
//! A preemptive scheduler needs interrupt handlers that save/restore full
//! CPU state — easy to get wrong, hard to review. Reviewers rightly reject
//! AI-generated context-switch assembly. This file has zero assembly and
//! zero `unsafe`, so any Rust developer can audit it in 5 minutes.
//!
//! PHASE 2 (planned, needs RFC + human review):
//! timer interrupt (APIC), per-task stacks, context switch.
//!
//! PURE BY DESIGN:
//! Demo steps here must NEVER print or touch hardware — printing lives in
//! the kernel (which owns the serial port). `heartbeat_step` is a silent
//! proof-of-liveness; the kernel's own logger task handles periodic output.
//!
//! HOST TESTS:
//! All logic is pure and tested on the host.

/// Maximum number of tasks. Fixed array = no heap, no allocator needed.
pub const MAX_TASKS: usize = 8;

/// A single cooperative task.
///
/// `step` is called once per scheduler tick. It must return quickly and
/// must never block: a task that loops forever starves everyone else.
/// This rule is documented (not enforced) in Phase 1 on purpose — enforcing
/// it needs preemption (Phase 2).
pub struct Task {
    /// Unique id assigned at spawn time (0, 1, 2, ...).
    pub id: usize,
    /// Short human name shown by the `tasks` shell command.
    pub name: &'static str,
    /// How many times this task has run. Doubles as uptime heartbeat.
    pub run_count: u64,
    /// The work to do each tick.
    pub step: fn(&mut Task),
}

impl Task {
    /// Create a task. `id` is assigned by the scheduler; callers pass a
    /// placeholder that `Scheduler::spawn` overwrites.
    pub const fn new(name: &'static str, step: fn(&mut Task)) -> Self {
        Self {
            id: 0,
            name,
            run_count: 0,
            step,
        }
    }
}

/// Round-robin cooperative scheduler.
pub struct Scheduler {
    tasks: [Option<Task>; MAX_TASKS],
    next_id: usize,
    /// Index of the task to run next. Wraps around.
    cursor: usize,
    /// Total ticks since boot. Drives the `uptime` shell command.
    pub ticks: u64,
}

impl Scheduler {
    /// Empty scheduler with no tasks.
    pub const fn new() -> Self {
        // `Option<Task>` is not `Copy`, so we cannot use `[None; N]`
        // in const context on older toolchains; explicit array it is.
        Self {
            tasks: [None, None, None, None, None, None, None, None],
            next_id: 0,
            cursor: 0,
            ticks: 0,
        }
    }

    /// Add a task. Returns its id, or `None` when the table is full.
    pub fn spawn(&mut self, mut task: Task) -> Option<usize> {
        for slot in self.tasks.iter_mut() {
            if slot.is_none() {
                let id = self.next_id;
                self.next_id += 1;
                task.id = id;
                *slot = Some(task);
                return Some(id);
            }
        }
        None
    }

    /// Number of live tasks.
    pub fn task_count(&self) -> usize {
        self.tasks.iter().filter(|t| t.is_some()).count()
    }

    /// Run the next ready task (one tick). Always advances `ticks`, even
    /// when no task is installed, so uptime keeps flowing before first spawn.
    pub fn tick(&mut self) {
        self.ticks += 1;

        // Find the next occupied slot starting at `cursor`, wrapping at most
        // MAX_TASKS steps. Borrow dance: take the task out, run it, put it
        // back — avoids holding a borrow across the `step` call.
        for offset in 0..MAX_TASKS {
            let index = (self.cursor + offset) % MAX_TASKS;
            if self.tasks[index].is_some() {
                // Move out, run, move back in.
                let mut task = self.tasks[index].take().expect("just checked Some");
                task.run_count += 1;
                (task.step)(&mut task);
                self.tasks[index] = Some(task);
                self.cursor = (index + 1) % MAX_TASKS;
                return;
            }
        }
        // No tasks installed: idle tick, just advance the cursor.
        self.cursor = (self.cursor + 1) % MAX_TASKS;
    }

    /// Iterate over live tasks for the `tasks` shell command.
    pub fn each_task(&self, f: impl FnMut(&Task)) {
        let mut f = f;
        for slot in self.tasks.iter().flatten() {
            f(slot);
        }
    }
}

impl Default for Scheduler {
    /// Default = empty scheduler. Required by Clippy (`new_without_default`):
    /// any `new()` without arguments must have a `Default` impl.
    fn default() -> Self {
        Self::new()
    }
}

/// Demo workload: silent heartbeat. Proves tasks actually run without
/// touching hardware or spamming any log. The scheduler already bumps
/// `run_count`; this step intentionally does nothing else.
pub fn heartbeat_step(task: &mut Task) {
    let _ = task.run_count;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(_: &mut Task) {}

    #[test]
    fn spawn_assigns_incremental_ids() {
        let mut s = Scheduler::new();
        let a = s.spawn(Task::new("a", noop)).unwrap();
        let b = s.spawn(Task::new("b", noop)).unwrap();
        assert_eq!((a, b), (0, 1));
        assert_eq!(s.task_count(), 2);
    }

    #[test]
    fn table_full_returns_none() {
        let mut s = Scheduler::new();
        for i in 0..MAX_TASKS {
            assert!(
                s.spawn(Task::new("t", noop)).is_some(),
                "slot {i} should fit"
            );
        }
        assert!(s.spawn(Task::new("overflow", noop)).is_none());
    }

    #[test]
    fn round_robin_runs_each_task_once_per_cycle() {
        let mut s = Scheduler::new();
        s.spawn(Task::new("a", noop)).unwrap();
        s.spawn(Task::new("b", noop)).unwrap();
        s.tick();
        s.tick();
        // no_std compatible: fixed array instead of Vec.
        let mut counts = [0u64; 2];
        let mut i = 0;
        s.each_task(|t| {
            counts[i] = t.run_count;
            i += 1;
        });
        assert_eq!(counts, [1, 1]);
        assert_eq!(s.ticks, 2);
    }

    #[test]
    fn idle_tick_advances_uptime_without_tasks() {
        let mut s = Scheduler::new();
        s.tick();
        assert_eq!(s.ticks, 1);
        assert_eq!(s.task_count(), 0);
    }

    #[test]
    fn heartbeat_step_is_side_effect_free() {
        let mut s = Scheduler::new();
        s.spawn(Task::new("hb", heartbeat_step)).unwrap();
        s.tick();
        let mut runs = 0;
        s.each_task(|t| runs = t.run_count);
        assert_eq!(runs, 1);
    }
}
