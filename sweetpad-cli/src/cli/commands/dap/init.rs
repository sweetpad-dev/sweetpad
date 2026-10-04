//! `sweetpad dap init --editor nvim|zed`: write the editor's entry for
//! `sweetpad dap`, next to the project.
//!
//! nvim-dap takes an adapter and its configurations from Lua, so nvim gets a
//! block in the project's `.nvim.lua`, which nvim reads with `exrc` on. Zed's
//! debug configurations live in `.zed/debug.json`, and its Swift adapter runs
//! whatever binary the user's settings name for it, with no arguments; the
//! `sweetpad-dap` name for this binary is what serves on stdio without one.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cli::output::Output;
use crate::cli::resolve;
use crate::cli::{CliError, CommandResult, Context, ErrorKind, Render, Rendered};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Editor {
    /// Neovim with nvim-dap.
    Nvim,
    /// Zed, through its Swift extension's debug adapter.
    Zed,
}

/// The name of the configuration `dap init` writes, which a later run finds
/// and replaces.
const CONFIG_NAME: &str = "SweetPad: Build and Run";

const NVIM_BEGIN: &str = "-- >>> sweetpad dap (written by 'sweetpad dap init')";
const NVIM_END: &str = "-- <<< sweetpad dap";

/// What `dap init` wrote, and what the user still has to do.
struct DapInit {
    editor: Editor,
    file: PathBuf,
    /// Settings the user adds to the editor's own configuration.
    settings: Option<Value>,
    notes: Vec<String>,
}

impl Render for DapInit {
    fn human(&self, out: &Output) {
        out.note(&format!("wrote {}", self.file.display()));
        for note in &self.notes {
            out.note(note);
        }
    }

    fn json(&self) -> Value {
        json!({
            "editor": match self.editor {
                Editor::Nvim => "nvim",
                Editor::Zed => "zed",
            },
            "file": self.file.display().to_string(),
            "settings": self.settings,
            "notes": self.notes,
        })
    }
}

pub(super) fn run(ctx: &mut Context, editor: Editor, output: Option<&Path>) -> CommandResult {
    let root = project_root(ctx)?;
    let exe = std::env::current_exe()
        .map_err(|e| CliError::new(format!("finding the sweetpad binary: {e}")))?;
    let report = match editor {
        Editor::Nvim => init_nvim(&root, output, &exe)?,
        Editor::Zed => init_zed(&root, output, &exe)?,
    };
    Ok(Rendered::data(report))
}

/// The directory the project's editor files go in: the container's, as for
/// `bsp init`, or the working directory outside a project.
fn project_root(ctx: &Context) -> Result<PathBuf, CliError> {
    let dir = match resolve::container_silently(ctx) {
        Some(container) => container
            .path()
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(Path::to_path_buf),
        None => None,
    };
    match dir {
        Some(dir) => Ok(dir),
        None => std::env::current_dir()
            .map_err(|e| CliError::new(format!("reading the working directory: {e}"))),
    }
}

fn init_nvim(root: &Path, output: Option<&Path>, exe: &Path) -> Result<DapInit, CliError> {
    let file = output.map_or_else(|| root.join(".nvim.lua"), Path::to_path_buf);
    let existing = std::fs::read_to_string(&file).unwrap_or_default();
    let text = with_nvim_block(&existing, &nvim_block(exe));
    write(&file, &text)?;
    Ok(DapInit {
        editor: Editor::Nvim,
        file,
        settings: None,
        notes: vec![
            "nvim reads it once 'exrc' is on (vim.o.exrc = true) and you trust the file; \
             start debugging with ':DapContinue'"
                .to_string(),
        ],
    })
}

/// The Lua that registers the adapter and a launch configuration for Swift,
/// Objective-C, C and C++ files. It does nothing where nvim-dap isn't
/// installed.
fn nvim_block(exe: &Path) -> String {
    let command = lua_string(&exe.display().to_string());
    format!(
        "{NVIM_BEGIN}\n\
         local ok, dap = pcall(require, \"dap\")\n\
         if ok then\n  \
           dap.adapters.sweetpad = {{ type = \"executable\", command = {command}, args = {{ \"dap\" }} }}\n  \
           for _, language in ipairs({{ \"swift\", \"objc\", \"c\", \"cpp\" }}) do\n    \
             local configurations = dap.configurations[language] or {{}}\n    \
             for i = #configurations, 1, -1 do\n      \
               if configurations[i].type == \"sweetpad\" then table.remove(configurations, i) end\n    \
             end\n    \
             table.insert(configurations, {{\n      \
               type = \"sweetpad\",\n      \
               request = \"launch\",\n      \
               name = \"{CONFIG_NAME}\",\n      \
               cwd = \"${{workspaceFolder}}\",\n    \
             }})\n    \
             dap.configurations[language] = configurations\n  \
           end\n\
         end\n\
         {NVIM_END}\n"
    )
}

/// `existing` with the sweetpad block replaced, or appended when it has none.
fn with_nvim_block(existing: &str, block: &str) -> String {
    if let Some(start) = existing.find(NVIM_BEGIN)
        && let Some(end) = existing[start..].find(NVIM_END)
    {
        let end = start + end + NVIM_END.len();
        let end = existing[end..].strip_prefix('\n').map_or(end, |_| end + 1);
        return format!("{}{block}{}", &existing[..start], &existing[end..]);
    }
    if existing.is_empty() {
        return block.to_string();
    }
    let separator = if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    format!("{existing}{separator}{block}")
}

/// A double-quoted Lua string literal.
fn lua_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn init_zed(root: &Path, output: Option<&Path>, exe: &Path) -> Result<DapInit, CliError> {
    let file = output.map_or_else(|| root.join(".zed").join("debug.json"), Path::to_path_buf);
    let scenario = json!({
        "label": CONFIG_NAME,
        "adapter": "Swift",
        "request": "launch",
        "cwd": "$ZED_WORKTREE_ROOT",
    });
    let scenarios = match std::fs::read_to_string(&file) {
        Ok(text) => with_zed_scenario(&text, scenario).map_err(|e| {
            CliError::new(format!(
                "{} isn't plain JSON ({e}), so it's left as it is; add a scenario with \
                 \"adapter\": \"Swift\" and \"request\": \"launch\" to it yourself",
                file.display()
            ))
            .kind(ErrorKind::Usage)
        })?,
        Err(_) => Value::Array(vec![scenario]),
    };
    let text = serde_json::to_string_pretty(&scenarios).unwrap_or_default() + "\n";
    write(&file, &text)?;
    let adapter = on_path("sweetpad-dap").unwrap_or_else(|| exe.with_file_name("sweetpad-dap"));
    let settings = json!({ "dap": { "Swift": { "binary": adapter.display().to_string() } } });
    let mut notes = vec![format!(
        "add to Zed's settings, so its Swift adapter runs sweetpad: {settings}"
    )];
    if !adapter.exists() {
        notes.push(format!(
            "{} doesn't exist yet: create it with 'ln -s {} {}' (Homebrew installs it)",
            adapter.display(),
            exe.display(),
            adapter.display()
        ));
    }
    Ok(DapInit {
        editor: Editor::Zed,
        file,
        settings: Some(settings),
        notes,
    })
}

/// `name` as found on `PATH`, which outlasts an upgrade where the path of
/// the running binary may not.
fn on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The scenarios in `text` with sweetpad's replaced, or appended when it has
/// none.
fn with_zed_scenario(text: &str, scenario: Value) -> Result<Value, String> {
    let Value::Array(mut scenarios) =
        serde_json::from_str::<Value>(text).map_err(|e| e.to_string())?
    else {
        return Err("expected a list of scenarios".to_string());
    };
    match scenarios
        .iter_mut()
        .find(|s| s["label"] == CONFIG_NAME && s["adapter"] == "Swift")
    {
        Some(existing) => *existing = scenario,
        None => scenarios.push(scenario),
    }
    Ok(Value::Array(scenarios))
}

fn write(file: &Path, text: &str) -> Result<(), CliError> {
    if let Some(parent) = file.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| CliError::new(format!("creating {}: {e}", parent.display())))?;
    }
    std::fs::write(file, text)
        .map_err(|e| CliError::new(format!("writing {}: {e}", file.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nvim_block_is_appended_once_and_then_replaced() {
        let block = nvim_block(Path::new("/opt/homebrew/bin/sweetpad"));
        let first = with_nvim_block("vim.o.number = true\n", &block);
        assert!(first.starts_with("vim.o.number = true\n\n-- >>> sweetpad dap"));
        let moved = nvim_block(Path::new("/usr/local/bin/sweetpad"));
        let second = with_nvim_block(&first, &moved);
        assert_eq!(second.matches(NVIM_BEGIN).count(), 1);
        assert!(second.contains("/usr/local/bin/sweetpad"));
        assert!(!second.contains("/opt/homebrew/bin/sweetpad"));
        assert!(second.starts_with("vim.o.number = true\n"));
    }

    #[test]
    fn nvim_block_registers_the_adapter_with_the_dap_argument() {
        let block = nvim_block(Path::new("/bin/sweet\"pad"));
        assert!(
            block.contains(r#"command = "/bin/sweet\"pad", args = { "dap" }"#),
            "{block}"
        );
        assert!(block.contains("cwd = \"${workspaceFolder}\""), "{block}");
    }

    #[test]
    fn zed_scenario_replaces_its_own_and_keeps_the_rest() {
        let text = r#"[{"label": "Other", "adapter": "CodeLLDB"},
                       {"label": "SweetPad: Build and Run", "adapter": "Swift", "request": "attach"}]"#;
        let merged = with_zed_scenario(
            text,
            json!({ "label": CONFIG_NAME, "adapter": "Swift", "request": "launch" }),
        )
        .unwrap();
        let list = merged.as_array().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0]["label"], "Other");
        assert_eq!(list[1]["request"], "launch");
    }

    #[test]
    fn zed_scenario_refuses_a_file_it_cannot_parse() {
        assert!(with_zed_scenario("// comment\n[]", json!({})).is_err());
        assert!(with_zed_scenario("{}", json!({})).is_err());
    }
}
