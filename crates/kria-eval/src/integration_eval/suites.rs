//! Integration eval suites — 10 NoDisplay, CI-safe test cases.
//!
//! All cases use only `sh`, `python3`, and (optionally) `rustc`.
//! No GUI, no daemon, no uinput required.
//!
//! Each suite function returns a `Vec<IntegrationEvalCase>`. The runner
//! executes them in a fresh temp dir per case.

use super::runner::IntegrationEvalCase;
use super::verifier::{checker_for, ObservableOutputChecker};

// ============================================================================
// Suite 1: Pascal Triangle
// ============================================================================

pub fn pascal_triangle() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "pascal_triangle".into(),
        file_name: "pascal.py".into(),
        file_content: r#"
def pascal(n):
    row = [1]
    for _ in range(n):
        print(' '.join(str(x) for x in row))
        row = [1] + [row[i] + row[i+1] for i in range(len(row)-1)] + [1]

pascal(6)
"#
        .into(),
        command: "python3 pascal.py".into(),
        checker: checker_for(&["1 1", "1 2 1", "1 3 3 1"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// Suite 2: Fibonacci
// ============================================================================

pub fn fibonacci() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "fibonacci".into(),
        file_name: "fib.py".into(),
        file_content: r#"
def fib(n):
    a, b = 0, 1
    for _ in range(n):
        print(a, end=' ')
        a, b = b, a + b
    print()

fib(10)
"#
        .into(),
        command: "python3 fib.py".into(),
        checker: checker_for(&["0 1 1 2 3 5 8 13 21 34"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// Suite 3: Hello World (Python)
// ============================================================================

pub fn hello_world_python() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "hello_world_python".into(),
        file_name: "hello.py".into(),
        file_content: "print('Hello, world!')\n".into(),
        command: "python3 hello.py".into(),
        checker: checker_for(&["Hello, world!"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// Suite 4: Hello World (Rust) — skipped when rustc is unavailable
// ============================================================================

pub fn hello_world_rust() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "hello_world_rust".into(),
        file_name: "hello.rs".into(),
        file_content: r#"
fn main() {
    println!("Hello, world!");
}
"#
        .into(),
        command: "rustc hello.rs -o hello && ./hello".into(),
        checker: checker_for(&["Hello, world!"]),
        timeout_sec: 60,
    }
}

// ============================================================================
// Suite 5: Write and Verify File Content
// ============================================================================

pub fn write_and_verify() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "write_and_verify".into(),
        file_name: "data.txt".into(),
        file_content: "kria integration eval marker\nsecond line\n".into(),
        command: "cat data.txt".into(),
        checker: checker_for(&["kria integration eval marker", "second line"]),
        timeout_sec: 5,
    }
}

// ============================================================================
// Suite 6: Bubble Sort (Python)
// ============================================================================

pub fn bubble_sort_python() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "bubble_sort_python".into(),
        file_name: "sort.py".into(),
        file_content: r#"
def bubble_sort(arr):
    n = len(arr)
    for i in range(n):
        for j in range(0, n - i - 1):
            if arr[j] > arr[j + 1]:
                arr[j], arr[j + 1] = arr[j + 1], arr[j]
    return arr

arr = [64, 34, 25, 12, 22, 11, 90]
print(bubble_sort(arr))
"#
        .into(),
        command: "python3 sort.py".into(),
        checker: checker_for(&["11, 12, 22, 25, 34, 64, 90"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// Suite 7: Run Bash Script
// ============================================================================

pub fn run_bash_script() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "run_bash_script".into(),
        file_name: "info.sh".into(),
        file_content: r#"#!/usr/bin/env bash
echo "KRIA eval script running"
echo "PWD: $(pwd)"
echo "Lines: $(echo -e 'a\nb\nc' | wc -l | tr -d ' ')"
"#
        .into(),
        command: "bash info.sh".into(),
        checker: checker_for(&["KRIA eval script running", "Lines: 3"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// Suite 8: File Line Count
// ============================================================================

pub fn file_line_count() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "file_line_count".into(),
        file_name: "lines.txt".into(),
        file_content: "one\ntwo\nthree\nfour\nfive\n".into(),
        command: "wc -l lines.txt | awk '{print $1}'".into(),
        checker: ObservableOutputChecker {
            expected_fragments: vec!["5".into()],
            expected_exit_zero: true,
            expected_stdout_non_empty: true,
            case_insensitive: false,
        },
        timeout_sec: 5,
    }
}

// ============================================================================
// Suite 9: Python Dict / JSON Operations
// ============================================================================

pub fn python_json_operations() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "python_json_operations".into(),
        file_name: "json_op.py".into(),
        file_content: r#"
import json

data = {"name": "KRIA", "version": 1, "features": ["eval", "automation"]}
serialized = json.dumps(data)
parsed = json.loads(serialized)
print(parsed["name"])
print(parsed["version"])
print(parsed["features"][0])
"#
        .into(),
        command: "python3 json_op.py".into(),
        checker: checker_for(&["KRIA", "1", "eval"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// Suite 10: Matrix Multiplication (Python)
// ============================================================================

pub fn matrix_multiply_python() -> IntegrationEvalCase {
    IntegrationEvalCase {
        name: "matrix_multiply_python".into(),
        file_name: "matmul.py".into(),
        file_content: r#"
def matmul(A, B):
    rows_A, cols_A = len(A), len(A[0])
    cols_B = len(B[0])
    C = [[0] * cols_B for _ in range(rows_A)]
    for i in range(rows_A):
        for j in range(cols_B):
            for k in range(cols_A):
                C[i][j] += A[i][k] * B[k][j]
    return C

A = [[1, 2], [3, 4]]
B = [[5, 6], [7, 8]]
result = matmul(A, B)
for row in result:
    print(row)
"#
        .into(),
        command: "python3 matmul.py".into(),
        checker: checker_for(&["19", "22", "43", "50"]),
        timeout_sec: 10,
    }
}

// ============================================================================
// All suites (excluding rustc which requires external toolchain)
// ============================================================================

/// Returns all CI-safe cases (no rustc dependency).
pub fn all_nodisplay_cases() -> Vec<IntegrationEvalCase> {
    vec![
        pascal_triangle(),
        fibonacci(),
        hello_world_python(),
        write_and_verify(),
        bubble_sort_python(),
        run_bash_script(),
        file_line_count(),
        python_json_operations(),
        matrix_multiply_python(),
    ]
}

/// Returns all cases including `hello_world_rust`. Only use when rustc is installed.
pub fn all_cases_with_rust() -> Vec<IntegrationEvalCase> {
    let mut cases = all_nodisplay_cases();
    cases.push(hello_world_rust());
    cases
}
