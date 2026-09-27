//! Generates everything derived from `content/resume.yaml`.
//!
//! Usage: `cargo gen [all|latex|readme|html|schema|check]` (alias for
//! `cargo run -p resume-gen --`). Generated files are committed; `check`
//! fails when one is out of date, so LaTeX builds never need Rust.

mod latex;
mod spans;
mod templates;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, bail};
use resume_model::Resume;

const CONTENT: &str = "content/resume.yaml";
const MAIN_TEX: &str = "latex/main.tex";

const USAGE: &str = "usage: cargo gen [all|latex|readme|html|schema|check]";

struct Output {
    name: &'static str,
    /// Relative to the repository root.
    path: &'static str,
    render: fn(&Resume) -> anyhow::Result<String>,
}

const OUTPUTS: &[Output] = &[
    Output {
        name: "latex",
        path: "latex/generated/resume-data.tex",
        render: latex::render,
    },
    Output {
        name: "readme",
        path: "README.md",
        render: templates::readme,
    },
    Output {
        name: "html",
        path: "3d-resume/web/plain.html",
        render: templates::plain_html,
    },
    Output {
        name: "schema",
        path: "content/resume.schema.json",
        render: |_| schema(),
    },
];

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<ExitCode> {
    let command = std::env::args().nth(1).unwrap_or_else(|| "all".to_owned());
    let root = repo_root();
    let resume = resume_model::load_file(root.join(CONTENT))?;

    let skill_problems = check_skill_references(&root, &resume)?;
    for warning in &skill_problems.warnings {
        eprintln!("warning: {warning}");
    }

    match command.as_str() {
        "check" => check(&root, &resume, &skill_problems.errors),
        "all" => {
            for output in OUTPUTS {
                write(&root, output, &resume)?;
            }
            report_errors(&skill_problems.errors)
        }
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(ExitCode::SUCCESS)
        }
        name => {
            let Some(output) = OUTPUTS.iter().find(|output| output.name == name) else {
                bail!("unknown command `{name}`\n{USAGE}");
            };
            write(&root, output, &resume)?;
            report_errors(&skill_problems.errors)
        }
    }
}

/// The workspace root, two levels above this crate.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn write(root: &Path, output: &Output, resume: &Resume) -> anyhow::Result<()> {
    let content = (output.render)(resume).with_context(|| format!("rendering {}", output.name))?;
    let path = root.join(output.path);
    if read_normalized(&path).as_deref() == Some(content.as_str()) {
        println!("unchanged {}", output.path);
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, content).with_context(|| format!("writing {}", output.path))?;
    println!("wrote     {}", output.path);
    Ok(())
}

fn check(root: &Path, resume: &Resume, errors: &[String]) -> anyhow::Result<ExitCode> {
    let mut stale = Vec::new();
    for output in OUTPUTS {
        let expected = (output.render)(resume)?;
        if read_normalized(&root.join(output.path)).as_deref() != Some(expected.as_str()) {
            stale.push(output.path);
        }
    }
    for path in &stale {
        eprintln!("error: {path} is out of date with {CONTENT}");
    }
    if !stale.is_empty() {
        eprintln!("run `cargo gen all` and commit the result");
    }
    let code = report_errors(errors)?;
    if stale.is_empty() && code == ExitCode::SUCCESS {
        println!("all generated files are up to date");
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}

fn report_errors(errors: &[String]) -> anyhow::Result<ExitCode> {
    for error in errors {
        eprintln!("error: {error}");
    }
    Ok(if errors.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// Reads a file with line endings normalized (Git may check out CRLF on Windows).
fn read_normalized(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.replace("\r\n", "\n"))
}

fn schema() -> anyhow::Result<String> {
    let generator = schemars::generate::SchemaSettings::draft07().into_generator();
    let schema = generator.into_root_schema_for::<resume_model::source::Source>();
    Ok(serde_json::to_string_pretty(&schema)? + "\n")
}

#[derive(Default)]
struct SkillProblems {
    errors: Vec<String>,
    warnings: Vec<String>,
}

/// `main.tex` places skill groups by id: unknown ids are errors, groups that
/// never appear in the PDF are warnings.
fn check_skill_references(root: &Path, resume: &Resume) -> anyhow::Result<SkillProblems> {
    let main_tex =
        read_normalized(&root.join(MAIN_TEX)).with_context(|| format!("reading {MAIN_TEX}"))?;
    let referenced = skill_references(&main_tex);
    let mut problems = SkillProblems::default();
    for id in &referenced {
        if resume.skill_group(id).is_none() {
            problems
                .errors
                .push(format!("{MAIN_TEX} references unknown skill group `{id}`"));
        }
    }
    for group in &resume.skills {
        if !referenced.contains(&group.id) {
            problems.warnings.push(format!(
                "skill group `{}` is not used in {MAIN_TEX}, so it is missing from the PDF",
                group.id
            ));
        }
    }
    Ok(problems)
}

/// Ids passed to the skill macros, e.g. `\ResumeSkillGroup{graphics}`.
fn skill_references(tex: &str) -> Vec<String> {
    let mut ids = Vec::new();
    for line in tex.lines() {
        let line = line.split('%').next().unwrap_or_default();
        for name in latex::SKILL_MACROS {
            let pattern = format!("\\{name}{{");
            for (start, _) in line.match_indices(&pattern) {
                let rest = &line[start + pattern.len()..];
                if let Some(id) = rest.split('}').next()
                    && !id.starts_with('#')
                    && !ids.iter().any(|known| known == id)
                {
                    ids.push(id.to_owned());
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_skill_references() {
        let tex = r"
\newcommand{\ResumeSkillGroup}[1]{\ResumeSkillName{#1}\ResumeSkillTags{#1}}
\ResumeSkillGroup{graphics} \ResumeSkillGroup{graphics}
% \ResumeSkillGroup{commented}
\cvsection{\ResumeSkillName{practices}}\ResumeSkillItems{practices}
";
        assert_eq!(skill_references(tex), ["graphics", "practices"]);
    }
}
