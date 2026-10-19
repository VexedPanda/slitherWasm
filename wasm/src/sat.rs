//! A small CDCL SAT solver (in the style of MiniSat): two watched literals, first-UIP clause learning with
//! minimization, VSIDS variable activity, phase saving, Luby restarts, and learnt clause database reduction. Clauses can
//! be added between solves (used to exclude loops one at a time).
//!
//! Literals are `2 * variable + negated`.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lbool {
    True,
    False,
    Undef,
}

#[derive(PartialEq, Eq, Debug)]
pub enum SolveStatus {
    Sat,
    Unsat,
    // The conflict budget ran out
    Unknown,
}

pub type Lit = u32;

#[inline]
pub fn lit(var: u32, negated: bool) -> Lit {
    2 * var + negated as u32
}

#[inline]
fn var(l: Lit) -> usize {
    (l >> 1) as usize
}

#[inline]
fn negate(l: Lit) -> Lit {
    l ^ 1
}

const NO_REASON: u32 = u32::MAX;

struct Clause {
    lits: Vec<Lit>,
    learnt: bool,
    activity: f32,
    deleted: bool,
}

#[derive(Clone, Copy)]
struct Watcher {
    clause: u32,
    // Another literal of the clause: if it's true, the clause is satisfied without looking at it
    blocker: Lit,
}

pub struct Sat {
    clauses: Vec<Clause>,
    free_clauses: Vec<u32>,
    watches: Vec<Vec<Watcher>>,
    assigns: Vec<Lbool>,
    level: Vec<u32>,
    reason: Vec<u32>,
    trail: Vec<Lit>,
    trail_lim: Vec<usize>,
    qhead: usize,
    activity: Vec<f64>,
    var_inc: f64,
    clause_inc: f32,
    polarity: Vec<bool>,
    heap: Vec<u32>,
    heap_index: Vec<u32>,
    seen: Vec<u8>,
    analyze_stack: Vec<Lit>,
    analyze_clear: Vec<Lit>,
    learnt_count: usize,
    max_learnts: f64,
    ok: bool,
    pub conflicts: u64,
}

const NOT_IN_HEAP: u32 = u32::MAX;

impl Sat {
    pub fn new(num_vars: usize) -> Sat {
        let mut sat = Sat {
            clauses: Vec::new(),
            free_clauses: Vec::new(),
            watches: vec![Vec::new(); 2 * num_vars],
            assigns: vec![Lbool::Undef; num_vars],
            level: vec![0; num_vars],
            reason: vec![NO_REASON; num_vars],
            trail: Vec::with_capacity(num_vars),
            trail_lim: Vec::new(),
            qhead: 0,
            activity: vec![0.0; num_vars],
            var_inc: 1.0,
            clause_inc: 1.0,
            polarity: vec![true; num_vars],
            heap: Vec::with_capacity(num_vars),
            heap_index: vec![NOT_IN_HEAP; num_vars],
            seen: vec![0; num_vars],
            analyze_stack: Vec::new(),
            analyze_clear: Vec::new(),
            learnt_count: 0,
            max_learnts: 0.0,
            ok: true,
            conflicts: 0,
        };
        for v in 0..num_vars as u32 {
            sat.heap_insert(v);
        }
        sat
    }

    pub fn num_vars(&self) -> usize {
        self.assigns.len()
    }

    #[inline]
    pub fn value_var(&self, v: u32) -> Lbool {
        self.assigns[v as usize]
    }

    #[inline]
    fn value(&self, l: Lit) -> Lbool {
        match self.assigns[var(l)] {
            Lbool::Undef => Lbool::Undef,
            Lbool::True => {
                if l & 1 == 0 {
                    Lbool::True
                } else {
                    Lbool::False
                }
            }
            Lbool::False => {
                if l & 1 == 0 {
                    Lbool::False
                } else {
                    Lbool::True
                }
            }
        }
    }

    #[inline]
    fn decision_level(&self) -> u32 {
        self.trail_lim.len() as u32
    }

    // ---- variable order heap (max-heap on activity) ----

    #[inline]
    fn heap_less(&self, a: u32, b: u32) -> bool {
        self.activity[a as usize] > self.activity[b as usize]
    }

    fn heap_up(&mut self, mut i: usize) {
        let v = self.heap[i];
        while i > 0 {
            let parent = (i - 1) / 2;
            if !self.heap_less(v, self.heap[parent]) {
                break;
            }
            self.heap[i] = self.heap[parent];
            self.heap_index[self.heap[i] as usize] = i as u32;
            i = parent;
        }
        self.heap[i] = v;
        self.heap_index[v as usize] = i as u32;
    }

    fn heap_down(&mut self, mut i: usize) {
        let v = self.heap[i];
        let n = self.heap.len();
        loop {
            let left = 2 * i + 1;
            if left >= n {
                break;
            }
            let right = left + 1;
            let child = if right < n && self.heap_less(self.heap[right], self.heap[left]) {
                right
            } else {
                left
            };
            if !self.heap_less(self.heap[child], v) {
                break;
            }
            self.heap[i] = self.heap[child];
            self.heap_index[self.heap[i] as usize] = i as u32;
            i = child;
        }
        self.heap[i] = v;
        self.heap_index[v as usize] = i as u32;
    }

    fn heap_insert(&mut self, v: u32) {
        if self.heap_index[v as usize] != NOT_IN_HEAP {
            return;
        }
        self.heap.push(v);
        let i = self.heap.len() - 1;
        self.heap_index[v as usize] = i as u32;
        self.heap_up(i);
    }

    fn heap_pop(&mut self) -> Option<u32> {
        if self.heap.is_empty() {
            return None;
        }
        let top = self.heap[0];
        let last = self.heap.pop().unwrap();
        self.heap_index[top as usize] = NOT_IN_HEAP;
        if !self.heap.is_empty() {
            self.heap[0] = last;
            self.heap_index[last as usize] = 0;
            self.heap_down(0);
        }
        Some(top)
    }

    fn bump_var(&mut self, v: usize) {
        self.activity[v] += self.var_inc;
        if self.activity[v] > 1e100 {
            for a in self.activity.iter_mut() {
                *a *= 1e-100;
            }
            self.var_inc *= 1e-100;
        }
        let i = self.heap_index[v];
        if i != NOT_IN_HEAP {
            self.heap_up(i as usize);
        }
    }

    fn bump_clause(&mut self, c: u32) {
        let clause = &mut self.clauses[c as usize];
        clause.activity += self.clause_inc;
        if clause.activity > 1e20 {
            for clause in self.clauses.iter_mut() {
                clause.activity *= 1e-20;
            }
            self.clause_inc *= 1e-20;
        }
    }

    // ---- clauses ----

    fn store_clause(&mut self, lits: Vec<Lit>, learnt: bool) -> u32 {
        let clause = Clause {
            lits,
            learnt,
            activity: 0.0,
            deleted: false,
        };
        let c = if let Some(c) = self.free_clauses.pop() {
            self.clauses[c as usize] = clause;
            c
        } else {
            self.clauses.push(clause);
            (self.clauses.len() - 1) as u32
        };
        let lits = &self.clauses[c as usize].lits;
        let (l0, l1) = (lits[0], lits[1]);
        self.watches[negate(l0) as usize].push(Watcher { clause: c, blocker: l1 });
        self.watches[negate(l1) as usize].push(Watcher { clause: c, blocker: l0 });
        if learnt {
            self.learnt_count += 1;
        }
        c
    }

    /// Adds a clause (at decision level 0). Returns false if the formula is now unsatisfiable.
    pub fn add_clause(&mut self, lits: &[Lit]) -> bool {
        if !self.ok {
            return false;
        }
        self.cancel_until(0);
        let mut clause: Vec<Lit> = Vec::with_capacity(lits.len());
        for &l in lits {
            match self.value(l) {
                Lbool::True => return true,
                Lbool::False => {}
                Lbool::Undef => {
                    if clause.contains(&negate(l)) {
                        return true;
                    }
                    if !clause.contains(&l) {
                        clause.push(l);
                    }
                }
            }
        }
        match clause.len() {
            0 => {
                self.ok = false;
                false
            }
            1 => {
                self.enqueue(clause[0], NO_REASON);
                if self.propagate() != NO_REASON {
                    self.ok = false;
                }
                self.ok
            }
            _ => {
                self.store_clause(clause, false);
                true
            }
        }
    }

    #[inline]
    fn enqueue(&mut self, l: Lit, reason: u32) {
        let v = var(l);
        self.assigns[v] = if l & 1 == 0 { Lbool::True } else { Lbool::False };
        self.level[v] = self.decision_level();
        self.reason[v] = reason;
        self.trail.push(l);
    }

    // Unit propagation. Returns the conflicting clause, or NO_REASON.
    fn propagate(&mut self) -> u32 {
        let mut conflict = NO_REASON;
        while self.qhead < self.trail.len() {
            let p = self.trail[self.qhead]; // p is true, so clauses watching ~p (stored at watches[p]) may need work
            self.qhead += 1;
            let false_lit = negate(p);
            let mut watchers = std::mem::take(&mut self.watches[p as usize]);
            let mut i = 0;
            let mut j = 0;
            while i < watchers.len() {
                let w = watchers[i];
                i += 1;
                if self.value(w.blocker) == Lbool::True {
                    watchers[j] = w;
                    j += 1;
                    continue;
                }
                let c = w.clause as usize;
                if self.clauses[c].deleted {
                    continue;
                }
                // Make sure the false literal is lits[1]
                {
                    let lits = &mut self.clauses[c].lits;
                    if lits[0] == false_lit {
                        lits.swap(0, 1);
                    }
                }
                let first = self.clauses[c].lits[0];
                let new_watcher = Watcher {
                    clause: w.clause,
                    blocker: first,
                };
                if first != w.blocker && self.value(first) == Lbool::True {
                    watchers[j] = new_watcher;
                    j += 1;
                    continue;
                }
                // Look for a new literal to watch
                let mut found = false;
                let len = self.clauses[c].lits.len();
                for k in 2..len {
                    let l = self.clauses[c].lits[k];
                    if self.value(l) != Lbool::False {
                        self.clauses[c].lits.swap(1, k);
                        self.watches[negate(l) as usize].push(new_watcher);
                        found = true;
                        break;
                    }
                }
                if found {
                    continue;
                }
                watchers[j] = new_watcher;
                j += 1;
                if self.value(first) == Lbool::False {
                    conflict = w.clause;
                    self.qhead = self.trail.len();
                    while i < watchers.len() {
                        watchers[j] = watchers[i];
                        j += 1;
                        i += 1;
                    }
                } else {
                    self.enqueue(first, w.clause);
                }
            }
            watchers.truncate(j);
            // Watchers added to this list during the loop (none: a clause never moves its watch to ~p's list here,
            // since that literal is false) are kept by appending
            let added = std::mem::take(&mut self.watches[p as usize]);
            watchers.extend(added);
            self.watches[p as usize] = watchers;
            if conflict != NO_REASON {
                break;
            }
        }
        conflict
    }

    fn cancel_until(&mut self, level: u32) {
        if self.decision_level() > level {
            let target = self.trail_lim[level as usize];
            for i in (target..self.trail.len()).rev() {
                let v = var(self.trail[i]);
                self.assigns[v] = Lbool::Undef;
                self.polarity[v] = self.trail[i] & 1 == 1;
                self.reason[v] = NO_REASON;
                self.heap_insert(v as u32);
            }
            self.trail.truncate(target);
            self.trail_lim.truncate(level as usize);
            self.qhead = target;
        }
    }

    // Whether literal p (in a learnt clause) is implied by the other literals, so it can be removed
    fn redundant(&mut self, p: Lit, abstract_levels: u32) -> bool {
        self.analyze_stack.clear();
        self.analyze_stack.push(p);
        let top = self.analyze_clear.len();
        while let Some(q) = self.analyze_stack.pop() {
            let c = self.reason[var(q)];
            let len = self.clauses[c as usize].lits.len();
            for k in 1..len {
                let l = self.clauses[c as usize].lits[k];
                let v = var(l);
                if self.seen[v] == 0 && self.level[v] > 0 {
                    if self.reason[v] != NO_REASON && (abstract_level(self.level[v]) & abstract_levels) != 0 {
                        self.seen[v] = 1;
                        self.analyze_stack.push(l);
                        self.analyze_clear.push(l);
                    } else {
                        for &l in &self.analyze_clear[top..] {
                            self.seen[var(l)] = 0;
                        }
                        self.analyze_clear.truncate(top);
                        return false;
                    }
                }
            }
        }
        true
    }

    // First-UIP conflict analysis. Returns the learnt clause (asserting literal first) and the backtrack level.
    fn analyze(&mut self, mut conflict: u32) -> (Vec<Lit>, u32) {
        let mut learnt: Vec<Lit> = vec![0];
        let mut path_count = 0;
        let mut p: Option<Lit> = None;
        let mut index = self.trail.len();

        loop {
            if self.clauses[conflict as usize].learnt {
                self.bump_clause(conflict);
            }
            let start = if p.is_some() { 1 } else { 0 };
            let len = self.clauses[conflict as usize].lits.len();
            for k in start..len {
                let q = self.clauses[conflict as usize].lits[k];
                let v = var(q);
                if self.seen[v] == 0 && self.level[v] > 0 {
                    self.bump_var(v);
                    self.seen[v] = 1;
                    if self.level[v] >= self.decision_level() {
                        path_count += 1;
                    } else {
                        learnt.push(q);
                    }
                }
            }
            // Next literal on the trail to look at
            loop {
                index -= 1;
                if self.seen[var(self.trail[index])] != 0 {
                    break;
                }
            }
            let next = self.trail[index];
            p = Some(next);
            conflict = self.reason[var(next)];
            self.seen[var(next)] = 0;
            path_count -= 1;
            if path_count == 0 {
                break;
            }
        }
        learnt[0] = negate(p.unwrap());

        // Minimize
        self.analyze_clear.clear();
        self.analyze_clear.extend_from_slice(&learnt);
        let mut abstract_levels = 0;
        for &l in &learnt[1..] {
            abstract_levels |= abstract_level(self.level[var(l)]);
        }
        let mut kept = 1;
        for i in 1..learnt.len() {
            let l = learnt[i];
            if self.reason[var(l)] == NO_REASON || !self.redundant(l, abstract_levels) {
                learnt[kept] = l;
                kept += 1;
            }
        }
        learnt.truncate(kept);
        for i in 0..self.analyze_clear.len() {
            let v = var(self.analyze_clear[i]);
            self.seen[v] = 0;
        }

        // Backtrack level: the highest level among the other literals (moved to position 1)
        let backtrack_level = if learnt.len() == 1 {
            0
        } else {
            let mut max_i = 1;
            for i in 2..learnt.len() {
                if self.level[var(learnt[i])] > self.level[var(learnt[max_i])] {
                    max_i = i;
                }
            }
            learnt.swap(1, max_i);
            self.level[var(learnt[1])]
        };
        (learnt, backtrack_level)
    }

    fn pick_branch(&mut self) -> Option<Lit> {
        while let Some(v) = self.heap_pop() {
            if self.assigns[v as usize] == Lbool::Undef {
                return Some(lit(v, self.polarity[v as usize]));
            }
        }
        None
    }

    fn reduce_learnts(&mut self) {
        let mut learnts: Vec<u32> = (0..self.clauses.len() as u32)
            .filter(|&c| {
                let clause = &self.clauses[c as usize];
                clause.learnt && !clause.deleted && clause.lits.len() > 2
            })
            .collect();
        learnts.sort_by(|&a, &b| {
            self.clauses[a as usize]
                .activity
                .partial_cmp(&self.clauses[b as usize].activity)
                .unwrap()
        });
        let remove = learnts.len() / 2;
        for &c in &learnts[..remove] {
            // Keep clauses that are currently the reason for an assignment
            let first = self.clauses[c as usize].lits[0];
            if self.reason[var(first)] == c && self.value(first) == Lbool::True {
                continue;
            }
            self.delete_clause(c);
        }
        // Drop watchers of deleted clauses
        for watchers in self.watches.iter_mut() {
            watchers.retain(|w| !self.clauses[w.clause as usize].deleted);
        }
        for c in 0..self.clauses.len() {
            if self.clauses[c].deleted && !self.clauses[c].lits.is_empty() {
                self.clauses[c].lits = Vec::new();
                self.free_clauses.push(c as u32);
            }
        }
    }

    fn delete_clause(&mut self, c: u32) {
        let clause = &mut self.clauses[c as usize];
        clause.deleted = true;
        if clause.learnt {
            self.learnt_count -= 1;
        }
    }

    /// Solves, giving up after `conflict_budget` more conflicts. On Sat, `model_value` gives the assignment.
    pub fn solve(&mut self, conflict_budget: u64) -> SolveStatus {
        if !self.ok {
            return SolveStatus::Unsat;
        }
        self.cancel_until(0);
        if self.propagate() != NO_REASON {
            self.ok = false;
            return SolveStatus::Unsat;
        }
        if self.max_learnts == 0.0 {
            self.max_learnts = (self.clauses.len() as f64 / 3.0).max(1000.0);
        }
        let budget_end = self.conflicts.saturating_add(conflict_budget);
        let mut restart = 0u32;
        loop {
            let limit = luby(2.0, restart) * 100.0;
            restart += 1;
            match self.search(limit as u64, budget_end) {
                Lbool::True => return SolveStatus::Sat,
                Lbool::False => {
                    self.ok = false;
                    return SolveStatus::Unsat;
                }
                Lbool::Undef => {
                    if self.conflicts >= budget_end {
                        self.cancel_until(0);
                        return SolveStatus::Unknown;
                    }
                }
            }
        }
    }

    fn search(&mut self, conflict_limit: u64, budget_end: u64) -> Lbool {
        let mut conflicts_here = 0u64;
        loop {
            let conflict = self.propagate();
            if conflict != NO_REASON {
                self.conflicts += 1;
                conflicts_here += 1;
                if self.decision_level() == 0 {
                    return Lbool::False;
                }
                let (learnt, backtrack_level) = self.analyze(conflict);
                self.cancel_until(backtrack_level);
                if learnt.len() == 1 {
                    self.enqueue(learnt[0], NO_REASON);
                } else {
                    let first = learnt[0];
                    let c = self.store_clause(learnt, true);
                    self.bump_clause(c);
                    self.enqueue(first, c);
                }
                self.var_inc *= 1.0 / 0.95;
                self.clause_inc *= 1.0 / 0.999;
            } else {
                if conflicts_here >= conflict_limit || self.conflicts >= budget_end {
                    self.cancel_until(0);
                    return Lbool::Undef;
                }
                if self.learnt_count as f64 - self.trail.len() as f64 >= self.max_learnts {
                    self.reduce_learnts();
                    self.max_learnts *= 1.1;
                }
                match self.pick_branch() {
                    None => return Lbool::True,
                    Some(l) => {
                        self.trail_lim.push(self.trail.len());
                        self.enqueue(l, NO_REASON);
                    }
                }
            }
        }
    }

    /// The value of a variable in the model found by the last successful solve (valid until the next change)
    pub fn model_value(&self, v: u32) -> bool {
        self.assigns[v as usize] == Lbool::True
    }

    /// Clears the assignment back to level 0 (after reading a model, before adding clauses)
    pub fn reset_to_root(&mut self) {
        self.cancel_until(0);
    }
}

#[inline]
fn abstract_level(level: u32) -> u32 {
    1 << (level & 31)
}

// The Luby restart sequence (1, 1, 2, 1, 1, 2, 4, ...) scaled by y
fn luby(y: f64, mut x: u32) -> f64 {
    let mut size = 1u32;
    let mut seq = 0u32;
    while size < x + 1 {
        seq += 1;
        size = 2 * size + 1;
    }
    while size - 1 != x {
        size = (size - 1) >> 1;
        seq -= 1;
        x %= size;
    }
    y.powi(seq as i32)
}
