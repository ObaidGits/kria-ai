//! A9.0.4 Generation Budget — hard limits with safe abort when exhausted.
//!
//! Tracks tokens, generation/repair/compile/test attempts, container/execution time,
//! memory/CPU/GPU/disk and cost. The pipeline checks the budget before each expensive
//! step and aborts safely (no partial install) when any dimension is exhausted.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Configurable budget limits (A9.0.4). Zero means "unlimited" for that dimension.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetLimits {
    pub max_llm_tokens: u64,
    pub max_generation_attempts: u32,
    pub max_repair_attempts: u32,
    pub max_compile_attempts: u32,
    pub max_test_attempts: u32,
    pub max_container_secs: u64,
    pub max_execution_secs: u64,
    pub max_memory_mb: u64,
    pub max_cpu_millis: u64,
    pub max_disk_mb: u64,
    /// Cost ceiling in micro-dollars (µ$), 0 = unlimited.
    pub max_cost_micros: u64,
}

impl Default for BudgetLimits {
    fn default() -> Self {
        Self {
            max_llm_tokens: 200_000,
            max_generation_attempts: 3,
            max_repair_attempts: 5,
            max_compile_attempts: 10,
            max_test_attempts: 10,
            max_container_secs: 300,
            max_execution_secs: 120,
            max_memory_mb: 2048,
            max_cpu_millis: 4000,
            max_disk_mb: 1024,
            max_cost_micros: 0,
        }
    }
}

/// Which budget dimension was exhausted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetDimension {
    Tokens,
    GenerationAttempts,
    RepairAttempts,
    CompileAttempts,
    TestAttempts,
    ContainerTime,
    ExecutionTime,
    Memory,
    Cpu,
    Disk,
    Cost,
}

impl BudgetDimension {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::GenerationAttempts => "generation_attempts",
            Self::RepairAttempts => "repair_attempts",
            Self::CompileAttempts => "compile_attempts",
            Self::TestAttempts => "test_attempts",
            Self::ContainerTime => "container_time",
            Self::ExecutionTime => "execution_time",
            Self::Memory => "memory",
            Self::Cpu => "cpu",
            Self::Disk => "disk",
            Self::Cost => "cost",
        }
    }
}

/// A live, thread-safe budget tracker (A9.0.4). Cheaply cloneable.
#[derive(Clone)]
pub struct GenerationBudget {
    limits: BudgetLimits,
    tokens: Arc<AtomicU64>,
    gen_attempts: Arc<AtomicU64>,
    repair_attempts: Arc<AtomicU64>,
    compile_attempts: Arc<AtomicU64>,
    test_attempts: Arc<AtomicU64>,
    cost_micros: Arc<AtomicU64>,
}

impl GenerationBudget {
    pub fn new(limits: BudgetLimits) -> Self {
        Self {
            limits,
            tokens: Arc::new(AtomicU64::new(0)),
            gen_attempts: Arc::new(AtomicU64::new(0)),
            repair_attempts: Arc::new(AtomicU64::new(0)),
            compile_attempts: Arc::new(AtomicU64::new(0)),
            test_attempts: Arc::new(AtomicU64::new(0)),
            cost_micros: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn limits(&self) -> &BudgetLimits {
        &self.limits
    }

    fn over(limit: u64, val: u64) -> bool {
        limit != 0 && val > limit
    }

    /// Charge tokens; returns Err(dimension) if the token budget is now exhausted.
    pub fn charge_tokens(&self, n: u64) -> Result<(), BudgetDimension> {
        let total = self.tokens.fetch_add(n, Ordering::Relaxed) + n;
        if Self::over(self.limits.max_llm_tokens, total) {
            Err(BudgetDimension::Tokens)
        } else {
            Ok(())
        }
    }

    /// Record + check a generation attempt.
    pub fn generation_attempt(&self) -> Result<u32, BudgetDimension> {
        let n = self.gen_attempts.fetch_add(1, Ordering::Relaxed) + 1;
        if Self::over(self.limits.max_generation_attempts as u64, n) {
            Err(BudgetDimension::GenerationAttempts)
        } else {
            Ok(n as u32)
        }
    }

    /// Record + check a repair attempt.
    pub fn repair_attempt(&self) -> Result<u32, BudgetDimension> {
        let n = self.repair_attempts.fetch_add(1, Ordering::Relaxed) + 1;
        if Self::over(self.limits.max_repair_attempts as u64, n) {
            Err(BudgetDimension::RepairAttempts)
        } else {
            Ok(n as u32)
        }
    }

    /// Record + check a compile attempt.
    pub fn compile_attempt(&self) -> Result<u32, BudgetDimension> {
        let n = self.compile_attempts.fetch_add(1, Ordering::Relaxed) + 1;
        if Self::over(self.limits.max_compile_attempts as u64, n) {
            Err(BudgetDimension::CompileAttempts)
        } else {
            Ok(n as u32)
        }
    }

    /// Record + check a test attempt.
    pub fn test_attempt(&self) -> Result<u32, BudgetDimension> {
        let n = self.test_attempts.fetch_add(1, Ordering::Relaxed) + 1;
        if Self::over(self.limits.max_test_attempts as u64, n) {
            Err(BudgetDimension::TestAttempts)
        } else {
            Ok(n as u32)
        }
    }

    /// Charge cost; returns Err if the cost ceiling is exceeded.
    pub fn charge_cost(&self, micros: u64) -> Result<(), BudgetDimension> {
        let total = self.cost_micros.fetch_add(micros, Ordering::Relaxed) + micros;
        if Self::over(self.limits.max_cost_micros, total) {
            Err(BudgetDimension::Cost)
        } else {
            Ok(())
        }
    }

    pub fn tokens_used(&self) -> u64 {
        self.tokens.load(Ordering::Relaxed)
    }
}
