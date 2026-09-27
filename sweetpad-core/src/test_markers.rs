//! The lines a test run prints to mark each test's progress, for XCTest and
//! Swift Testing, in every form `xcodebuild` and the test processes write
//! them. Checked against Xcode 27.0:
//!
//! - XCTest in a test process's own output, and in `xcodebuild`'s for a
//!   serial run: `Test Case '-[Module.Class method]' started.`, then `passed
//!   (0.001 seconds).`, `failed (…).` or `skipped (…).`, under `Test Suite
//!   'Class' started at <date>.`
//! - `xcodebuild`'s summary of a parallel run, for both frameworks: `Test case
//!   'Class.method()' passed on 'My Mac - xctest (123)' (0.206 seconds)`, with
//!   `Suite/function()` for a Swift Testing test, under `Test suite 'Class'
//!   started on '…'`.
//! - Swift Testing's own console output in a serial run: `◇ Test name()
//!   started.`, `✔ Test name() passed after 0.001 seconds.`, `✘ Test "Display
//!   name" failed after 0.001 seconds with 1 issue.`, `➜ Test name()
//!   skipped: "reason"`, and `◇ Suite Name started.`. The symbol varies (SF
//!   Symbols in Xcode's own console), and a line can open with a zero-width
//!   space.

/// How far a test case has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Started,
    Passed,
    Failed,
    Skipped,
}

/// Which of the marker forms a line is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    /// XCTest's `Test Case '-[Module.Class method]' …`, the form a test
    /// process writes around each test's own output.
    XCTest,
    /// `xcodebuild`'s `Test case '…' … on '<runner>' (…)` for a parallel run.
    Parallel,
    /// Swift Testing's own `Test … passed after …` lines.
    SwiftTesting,
}

/// One test case's marker, read apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseMarker<'a> {
    pub form: Form,
    pub status: Status,
    /// The module qualifying the class, when the marker spells it
    /// (`-[Module.Class method]`; an Objective-C class has none).
    pub module: Option<&'a str>,
    /// The class or suite, when the line names one, with any outer types
    /// (`Outer.InnerTests`, `Outer/InnerSuite`).
    pub class: Option<&'a str>,
    /// The method (`testPasses`, without parentheses for XCTest), the Swift
    /// Testing function (`passing()`), or its display name without quotes.
    pub name: &'a str,
    /// How long the test took, in seconds as printed (`0.001`).
    pub seconds: Option<&'a str>,
}

impl CaseMarker<'_> {
    /// The test's name for a person or a log: `Module.Class.method` for
    /// XCTest, `Suite/function()` for Swift Testing, and as much of either as
    /// the line gives.
    #[must_use]
    pub fn display_name(&self) -> String {
        let Some(class) = self.class else {
            return self.name.to_string();
        };
        // A Swift Testing function keeps its parentheses, and its suite joins
        // it with a slash, as the parallel form spells it.
        let separator = if self.form != Form::XCTest && self.name.ends_with(')') {
            '/'
        } else {
            '.'
        };
        match self.module {
            Some(module) => format!("{module}.{class}{separator}{}", self.name),
            None => format!("{class}{separator}{}", self.name),
        }
    }
}

/// Read a test case's marker off `line`: the start or end of one test, in
/// any of the forms in this module's docs. `None` for anything else,
/// including a run's or a suite's own start and end.
#[must_use]
pub fn parse_case(line: &str) -> Option<CaseMarker<'_>> {
    let t = trim(line);
    if let Some(rest) = t.strip_prefix("Test Case '-[") {
        return xctest_case(rest);
    }
    if let Some(rest) = t.strip_prefix("Test Case '") {
        return unbracketed_case(rest);
    }
    if let Some(rest) = t.strip_prefix("Test case '") {
        return parallel_case(rest);
    }
    swift_testing_case(t)
}

/// The suite a line says has started: XCTest's `Test Suite 'Name' started
/// at …`, a parallel run's `Test suite 'Name' started on '…'`, and Swift
/// Testing's `◇ Suite Name started.`.
#[must_use]
pub fn parse_suite_started(line: &str) -> Option<&str> {
    let t = trim(line);
    if let Some(rest) = t
        .strip_prefix("Test Suite '")
        .or_else(|| t.strip_prefix("Test suite '"))
    {
        let (name, tail) = rest.split_once("' ")?;
        return (tail.starts_with("started at ") || tail.starts_with("started on '"))
            .then_some(name);
    }
    let rest = after_symbol(t)?.strip_prefix("Suite ")?;
    let (name, tail) = split_name(rest)?;
    (tail == " started.").then_some(name)
}

/// Whether `line` is a test run's own output rather than its build step's:
/// a case or suite marker in any form, or Swift Testing's `◇ Test run
/// started.`.
#[must_use]
pub fn is_test_output(line: &str) -> bool {
    parse_case(line).is_some()
        || parse_suite_started(line).is_some()
        || after_symbol(trim(line)).is_some_and(|rest| rest == "Test run started.")
}

/// `line` without surrounding whitespace, nor the zero-width space Swift
/// Testing can open a line with.
fn trim(line: &str) -> &str {
    line.trim().trim_start_matches('\u{200B}').trim_start()
}

/// `Module.Class method]' passed (0.001 seconds).`, after `Test Case '-[`.
fn xctest_case(rest: &str) -> Option<CaseMarker<'_>> {
    let (inner, tail) = rest.split_once("]' ")?;
    let (qualified, method) = inner.split_once(' ')?;
    let status = status_word(tail)?;
    // The module is the first part: a nested class keeps its outer type
    // (`Module.Outer.InnerTests`).
    let (module, class) = match qualified.split_once('.') {
        Some((module, class)) => (Some(module), class),
        None => (None, qualified),
    };
    Some(CaseMarker {
        form: Form::XCTest,
        status,
        module,
        class: Some(class),
        name: method,
        seconds: paren_seconds(tail),
    })
}

/// `Class.method' passed (0.001 seconds).`, after `Test Case '`: the
/// open-source XCTest's spelling, which `swift test` prints off Apple
/// platforms.
fn unbracketed_case(rest: &str) -> Option<CaseMarker<'_>> {
    let (test, tail) = rest.split_once("' ")?;
    let (class, method) = test.rsplit_once('.')?;
    Some(CaseMarker {
        form: Form::XCTest,
        status: status_word(tail)?,
        module: None,
        class: Some(class),
        name: method,
        seconds: paren_seconds(tail),
    })
}

/// `Class.method()' passed on 'My Mac - xctest (123)' (0.206 seconds)`,
/// after `Test case '`. XCTest names the test `Class.method()`, Swift
/// Testing `Suite/function()` or a bare `function()`.
fn parallel_case(rest: &str) -> Option<CaseMarker<'_>> {
    let (test, tail) = rest.split_once("' ")?;
    let status = status_word(tail)?;
    if status == Status::Started {
        return None;
    }
    let (class, name) = match test.rsplit_once('/') {
        Some((suite, function)) => (Some(suite), function),
        None => match test.rsplit_once('.') {
            Some((class, method)) => (Some(class), method.strip_suffix("()").unwrap_or(method)),
            None => (None, test),
        },
    };
    Some(CaseMarker {
        form: Form::Parallel,
        status,
        module: None,
        class,
        name,
        seconds: tail.rsplit_once(" (").and_then(|(_, t)| seconds_in(t)),
    })
}

/// Swift Testing's `✔ Test name() passed after 0.001 seconds.` and its
/// siblings. The run's own `Test run …` lines and a parameterized test's
/// per-argument `Test case passing …` lines are not a test's.
fn swift_testing_case(t: &str) -> Option<CaseMarker<'_>> {
    let rest = after_symbol(t)?.strip_prefix("Test ")?;
    if rest.starts_with("run started.") || rest.starts_with("run with ") {
        return None;
    }
    let (name, tail) = split_name(rest)?;
    // A parameterized test ends as `Test name() with 2 test cases passed …`.
    let tail = match tail.strip_prefix(" with ") {
        Some(counted) => counted
            .split_once(" test case")
            .and_then(|(_, t)| t.strip_prefix('s').or(Some(t)))?,
        None => tail,
    };
    let (status, seconds) = if tail == " started." {
        (Status::Started, None)
    } else if let Some(time) = tail.strip_prefix(" passed after ") {
        (Status::Passed, seconds_in(time))
    } else if let Some(time) = tail.strip_prefix(" failed after ") {
        (Status::Failed, seconds_in(time))
    } else if tail == " skipped." || tail.starts_with(" skipped: ") {
        (Status::Skipped, None)
    } else {
        return None;
    };
    Some(CaseMarker {
        form: Form::SwiftTesting,
        status,
        module: None,
        class: None,
        name,
        seconds,
    })
}

/// A Swift Testing line after its leading symbol: the first word is
/// anything but a letter or digit.
fn after_symbol(t: &str) -> Option<&str> {
    let (symbol, rest) = t.split_once(' ')?;
    (!symbol.is_empty() && !symbol.chars().any(char::is_alphanumeric)).then(|| rest.trim_start())
}

/// A Swift Testing name and what follows it: a quoted display name, or a
/// function name up to the next space.
fn split_name(rest: &str) -> Option<(&str, &str)> {
    if let Some(quoted) = rest.strip_prefix('"') {
        let (name, tail) = quoted.split_once('"')?;
        return Some((name, tail));
    }
    let end = rest.find(' ')?;
    Some((&rest[..end], &rest[end..]))
}

/// The status an XCTest marker's tail opens with.
fn status_word(tail: &str) -> Option<Status> {
    [
        ("started", Status::Started),
        ("passed", Status::Passed),
        ("failed", Status::Failed),
        ("skipped", Status::Skipped),
    ]
    .into_iter()
    .find_map(|(word, status)| tail.starts_with(word).then_some(status))
}

/// `0.001` out of a tail holding `(0.001 seconds)`.
fn paren_seconds(tail: &str) -> Option<&str> {
    seconds_in(tail.split_once('(')?.1)
}

/// `0.001` out of `0.001 seconds…`.
fn seconds_in(text: &str) -> Option<&str> {
    let (number, _) = text.split_once(" second")?;
    number.parse::<f64>().is_ok().then_some(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(line: &str) -> (Status, Option<&str>, Option<&str>, &str, Option<&str>) {
        let m = parse_case(line).unwrap_or_else(|| panic!("no marker in {line:?}"));
        (m.status, m.module, m.class, m.name, m.seconds)
    }

    /// Lines from a serial `xcodebuild test` run on Xcode 27.0.
    #[test]
    fn reads_xctest_markers() {
        assert_eq!(
            case("Test Case '-[B10TMacTests.AlphaTests testFails]' started."),
            (
                Status::Started,
                Some("B10TMacTests"),
                Some("AlphaTests"),
                "testFails",
                None
            )
        );
        assert_eq!(
            case("Test Case '-[B10TMacTests.AlphaTests testFails]' failed (0.388 seconds)."),
            (
                Status::Failed,
                Some("B10TMacTests"),
                Some("AlphaTests"),
                "testFails",
                Some("0.388")
            )
        );
        assert_eq!(
            case("Test Case '-[B10TMacTests.BetaTests testSkipped]' skipped (0.002 seconds)."),
            (
                Status::Skipped,
                Some("B10TMacTests"),
                Some("BetaTests"),
                "testSkipped",
                Some("0.002")
            )
        );
        // An Objective-C class carries no module.
        assert_eq!(
            case("Test Case '-[ObjCTests testThing]' passed (0.001 seconds)."),
            (
                Status::Passed,
                None,
                Some("ObjCTests"),
                "testThing",
                Some("0.001")
            )
        );
        let m = parse_case("Test Case '-[M.A testA]' passed (0.1 seconds).").unwrap();
        assert_eq!(m.form, Form::XCTest);
        assert_eq!(m.display_name(), "M.A.testA");
        // The open-source XCTest's spelling, which has no brackets.
        assert_eq!(
            case("Test Case 'LoginTests.testFoo' passed (0.001 seconds)."),
            (
                Status::Passed,
                None,
                Some("LoginTests"),
                "testFoo",
                Some("0.001")
            )
        );
        // A nested class keeps its outer type under the module.
        let nested = parse_case("Test Case '-[M.Outer.InnerTests testX]' started.").unwrap();
        assert_eq!(
            (nested.module, nested.class),
            (Some("M"), Some("Outer.InnerTests"))
        );
        assert_eq!(nested.display_name(), "M.Outer.InnerTests.testX");
    }

    /// Lines from `xcodebuild test -parallel-testing-enabled YES` on Xcode
    /// 27.0, where both frameworks report in `xcodebuild`'s own form.
    #[test]
    fn reads_parallel_markers() {
        let on = "on 'My Mac - xctest (27841)'";
        assert_eq!(
            case(&format!(
                "Test case 'BetaTests.testPassesToo()' passed {on} (0.206 seconds)"
            )),
            (
                Status::Passed,
                None,
                Some("BetaTests"),
                "testPassesToo",
                Some("0.206")
            )
        );
        assert_eq!(
            case(&format!(
                "Test case 'AlphaTests.testFails()' failed {on} (0.422 seconds)"
            )),
            (
                Status::Failed,
                None,
                Some("AlphaTests"),
                "testFails",
                Some("0.422")
            )
        );
        assert_eq!(
            case(&format!(
                "Test case 'BetaTests.testSkipped()' skipped {on} (0.001 seconds)"
            )),
            (
                Status::Skipped,
                None,
                Some("BetaTests"),
                "testSkipped",
                Some("0.001")
            )
        );
        assert_eq!(
            case(&format!(
                "Test case 'GammaSuite/failing()' failed {on} (0.000 seconds)"
            )),
            (
                Status::Failed,
                None,
                Some("GammaSuite"),
                "failing()",
                Some("0.000")
            )
        );
        assert_eq!(
            case(&format!(
                "Test case 'freeFunction()' passed {on} (0.000 seconds)"
            )),
            (Status::Passed, None, None, "freeFunction()", Some("0.000"))
        );
        // A simulator clone names its runner with parentheses of its own.
        let sim = "Test case 'AppTests.testExample()' passed on 'Clone 1 of iPhone 17 - App (27767)' \
                   (0.254 seconds)";
        assert_eq!(case(sim).4, Some("0.254"));

        let display = |line: String| parse_case(&line).unwrap().display_name();
        assert_eq!(
            display(format!(
                "Test case 'BetaTests.testPassesToo()' passed {on} (0.2 seconds)"
            )),
            "BetaTests.testPassesToo"
        );
        assert_eq!(
            display(format!(
                "Test case 'GammaSuite/passing()' passed {on} (0.0 seconds)"
            )),
            "GammaSuite/passing()"
        );
    }

    /// Swift Testing's own lines from a serial run on Xcode 27.0, zero-width
    /// spaces included.
    #[test]
    fn reads_swift_testing_markers() {
        assert_eq!(
            case("◇ Test passing() started."),
            (Status::Started, None, None, "passing()", None)
        );
        assert_eq!(
            case("✔ Test passing() passed after 0.001 seconds."),
            (Status::Passed, None, None, "passing()", Some("0.001"))
        );
        assert_eq!(
            case("✘ Test \"Named failing test\" failed after 0.001 seconds with 1 issue."),
            (
                Status::Failed,
                None,
                None,
                "Named failing test",
                Some("0.001")
            )
        );
        assert_eq!(
            case(
                "\u{200B}✔ Test parameterized(value:) with 2 test cases passed after 0.001 seconds."
            ),
            (
                Status::Passed,
                None,
                None,
                "parameterized(value:)",
                Some("0.001")
            )
        );
        assert_eq!(
            case("\u{200B}➜ Test disabled() skipped: \"off\""),
            (Status::Skipped, None, None, "disabled()", None)
        );
        assert_eq!(
            case("➜ Test disabled() skipped."),
            (Status::Skipped, None, None, "disabled()", None)
        );
        // Xcode's console spells the symbols with SF Symbols.
        assert_eq!(
            case("\u{10105B} Test passing() passed after 0.001 seconds."),
            (Status::Passed, None, None, "passing()", Some("0.001"))
        );
        // A test named `run()` is a test; the run's own lines are not.
        assert_eq!(case("✔ Test run() passed after 0.001 seconds.").3, "run()");
        for not_a_case in [
            "◇ Test run started.",
            "✘ Test run with 5 tests in 1 suite failed after 0.002 seconds with 1 issue.",
            "✔ Test run with 1 test passed after 0.001 seconds.",
            "◇ Test case passing 1 argument value → 1 to parameterized(value:) started.",
            "✘ Test \"Named failing test\" recorded an issue at SwiftTestingTests.swift:9:9: \
             Expectation failed: 2 * 2 == 5",
            "\u{200B}✘ Suite GammaSuite failed after 0.001 seconds with 1 issue.",
            "◇ Suite GammaSuite started.",
            "Test Suite 'AlphaTests' started at 2026-09-27 19:11:06.251.",
            "Test session results, code coverage, and logs:",
            "Testing started",
        ] {
            assert_eq!(parse_case(not_a_case), None, "{not_a_case}");
        }
    }

    #[test]
    fn reads_suite_starts() {
        assert_eq!(
            parse_suite_started("Test Suite 'All tests' started at 2026-09-27 19:11:06.251."),
            Some("All tests")
        );
        assert_eq!(
            parse_suite_started("Test suite 'AlphaTests' started on 'My Mac - xctest (27838)'"),
            Some("AlphaTests")
        );
        assert_eq!(
            parse_suite_started("◇ Suite GammaSuite started."),
            Some("GammaSuite")
        );
        assert_eq!(
            parse_suite_started("◇ Suite \"Display Suite\" started."),
            Some("Display Suite")
        );
        for not_a_start in [
            "Test Suite 'AlphaTests' failed at 2026-09-27 19:11:06.641.",
            "✘ Suite GammaSuite failed after 0.001 seconds with 1 issue.",
            "◇ Test passing() started.",
        ] {
            assert_eq!(parse_suite_started(not_a_start), None, "{not_a_start}");
        }
    }

    #[test]
    fn test_output_starts_at_any_marker() {
        for line in [
            "Test Suite 'All tests' started at 2026-09-27 19:11:06.251.",
            "Test suite 'AlphaTests' started on 'My Mac - xctest (27838)'",
            "Test Case '-[M.A testA]' started.",
            "Test case 'A.testA()' passed on 'My Mac - xctest (1)' (0.1 seconds)",
            "◇ Test run started.",
            "􀟈 Test run started.",
        ] {
            assert!(is_test_output(line), "{line}");
        }
        for line in [
            "CompileSwift normal arm64 /src/Tests.swift",
            "** TEST FAILED **",
            "Testing started",
        ] {
            assert!(!is_test_output(line), "{line}");
        }
    }
}
