use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(args).output().unwrap()
}

fn stdout(o: &Output) -> String {
    assert!(o.status.success(), "exit {:?}\nstderr: {}", o.status.code(), String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    assert!(!o.status.success(), "exit {:?}\nstdout: {}", o.status.code(), String::from_utf8_lossy(&o.stdout));
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn words(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

const SUBCOMMANDS: [&str; 11] = ["convert", "info", "text", "inspect", "render", "run", "commands", "describe", "parity", "mcp", "zotero"];

#[test]
fn help_lists_every_subcommand_with_a_run_example() {
    for args in [&["help"][..], &["--help"], &["-h"]] {
        let out = stdout(&cli(args));
        for sub in SUBCOMMANDS {
            assert!(out.contains(&format!("\n  {sub} ")), "{args:?} lacks {sub}:\n{out}");
        }
        assert!(out.contains("wordcraft-cli run --cmd 'text.insert={\"text\":\"Hello\"}' --save hello.docx"), "{out}");
    }
}

#[test]
fn every_subcommand_answers_help_and_does_nothing_else() {
    let dir = std::env::temp_dir().join(format!("wc-cli-help-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for sub in SUBCOMMANDS {
        for args in [vec![sub, "--help"], vec![sub, "-h", "out.png"], vec!["help", sub]] {
            let o = Command::new(env!("CARGO_BIN_EXE_wordcraft-cli")).args(&args).current_dir(&dir).output().unwrap();
            let out = stdout(&o);
            assert!(out.starts_with(&format!("usage: wordcraft-cli {sub}")), "{args:?}:\n{out}");
        }
    }
    assert!(std::fs::read_dir(&dir).unwrap().next().is_none(), "help wrote a file");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_help_says_what_the_ids_do_not() {
    let out = words(&stdout(&cli(&["run", "--help"])));
    for fact in [
        "A document is a list of blocks, paragraphs and tables, numbered from 0.",
        "N is a byte offset into the paragraph's text",
        "Typing and inserting happen at the caret, replacing the selected text if there is one.",
        "format.* and para.* act on the selected text, else at the caret",
        "Sizes and distances are in points (72 pt = 1 inch); colours are \"RRGGBB\".",
        "If a command fails, run stops there and saves nothing.",
        "text.newParagraph carries the formatting at the caret into the new paragraph",
        "OK: 6 commands ran, saved report.docx",
        "--print shows each command's answer first",
    ] {
        assert!(out.contains(fact), "missing: {fact}\n{out}");
    }
}

#[test]
fn an_unknown_subcommand_names_the_real_ones() {
    let err = stderr(&cli(&["frobnicate"]));
    assert_eq!(
        err,
        "wordcraft-cli: `frobnicate` is not a subcommand of wordcraft-cli; the subcommands are convert, info, text, inspect, render, run, commands, describe, parity, mcp or zotero. `wordcraft-cli help` explains them.\n"
    );
    assert!(stderr(&cli(&["help", "frobnicate"])).contains("`frobnicate` is not a subcommand"));
    assert!(stderr(&cli(&[])).starts_with("wordcraft-cli: wordcraft-cli needs a subcommand.\n\nwordcraft-cli works on"));
}

#[test]
fn describe_spells_out_a_command() {
    assert_eq!(
        stdout(&cli(&["describe", "insert.picture"])),
        "insert.picture: Pictures, in Insert › Illustrations.\n\nParameters:\n  path, optional   string\n  data, optional   base64\n  width, optional  pt\n  alt, optional    string\n\nRun it with: wordcraft-cli run --cmd 'insert.picture={…}'\n"
    );
    assert_eq!(
        stdout(&cli(&["describe", "format.size"])),
        "format.size: Font Size, in Home › Font, shortcut Mod+Shift+P.\n\nParameters:\n  size  number\n\nRun it with: wordcraft-cli run --cmd 'format.size={…}'\n"
    );
    assert_eq!(
        stdout(&cli(&["describe", "edit.undo"])),
        "edit.undo: Undo, in Quick Access Toolbar, shortcut Mod+Z.\n\nTakes no parameters.\n\nRun it with: wordcraft-cli run --cmd edit.undo\n"
    );
    let out = stdout(&cli(&["describe", "caret.set"]));
    assert!(out.contains("  either:\n    pos               Pos\n    extend, optional  bool\n  or:\n    page              n\n"), "{out}");
    assert!(stdout(&cli(&["describe", "picture.size"])).contains("\nNot available in a new document: select a picture or shape first. insert.picture or insert.shape inserts one and selects it; select.objects {\"index\": n} selects an existing one.\n"));
    assert!(stdout(&cli(&["describe", "table.deleteRow"])).contains("\nNot available in a new document: the cursor isn't in a table. insert.table"));
    assert!(stdout(&cli(&["describe", "edit.advancedFind"])).contains("  and options of edit.find\n"));
    assert!(stdout(&cli(&["describe", "styles.modify"])).contains("\nCharProps, ParaProps and Pos: `wordcraft-cli help types`.\n"));
    let types = stdout(&cli(&["help", "types"]));
    assert!(types.contains("CharProps (format.set") && types.contains("ParaProps (para.set") && types.contains("Pos, a position"), "{types}");
    assert!(stderr(&cli(&["describe", "new"])).starts_with("wordcraft-cli: `new` is not a command; commands named like it: file.new, "));
}

#[test]
fn commands_lists_one_line_per_command() {
    let out = stdout(&cli(&["commands", "header"]));
    assert!(out.contains("insert.header — Header: {\"text\"?: string, \"preset\"?: \"blank|blankThree|title\"}\n"), "{out}");
    assert!(out.contains("insert.editHeader — Edit Header\n"), "{out}");
    assert!(!out.contains("format.bold"));
    assert!(stdout(&cli(&["commands"])).lines().count() > 600);
    let json: serde_json::Value = serde_json::from_str(&stdout(&cli(&["commands", "header", "--json"]))).unwrap();
    assert!(
        json.as_array()
            .unwrap()
            .iter()
            .all(|c| c["id"].as_str().unwrap().to_lowercase().contains("header") || c["label"].as_str().unwrap().to_lowercase().contains("header"))
    );
}

#[test]
fn a_label_before_the_command_is_an_unknown_command() {
    let err = stderr(&cli(&["run", "--cmd", "new=file.new={\"template\":\"blank\"}"]));
    assert!(err.starts_with("wordcraft-cli: --cmd 1 of 1 was read as command `new` with parameters `file.new={\"template\":\"blank\"}`. `new` is not a command; commands named like it: file.new, "), "{err}");
    assert!(err.ends_with(". --cmd takes COMMAND=PARAMS, a command id, then `=`, then a JSON object, e.g. --cmd 'text.insert={\"text\":\"Hello\"}'; `wordcraft-cli commands` lists the ids. Nothing ran.\n"), "{err}");
}

#[test]
fn bad_json_names_the_command_and_the_text() {
    let err = stderr(&cli(&["run", "--cmd", "text.insert={\"text\":\"Hi\",}"]));
    assert!(
        err.starts_with(
            "wordcraft-cli: --cmd 1 of 1 was read as command `text.insert` with parameters `{\"text\":\"Hi\",}`, which are not JSON: trailing comma"
        ),
        "{err}"
    );
    assert!(err.ends_with("`text.insert` takes {\"text\": string, \"raw\"?: bool (no AutoCorrect or AutoFormat)}. Nothing ran.\n"), "{err}");
}

#[test]
fn an_unknown_parameter_is_refused() {
    assert_eq!(
        stderr(&cli(&["run", "--cmd", "text.insert={\"content\":\"Hi\"}"])),
        "wordcraft-cli: --cmd 1 of 1: `text.insert` was given `content`, which it doesn't take. `text.insert` takes {\"text\": string, \"raw\"?: bool (no AutoCorrect or AutoFormat)}.\n"
    );
}

#[test]
fn a_failing_command_says_which_and_that_nothing_was_saved() {
    let out = std::env::temp_dir().join(format!("wc-cli-unsaved-{}.docx", std::process::id()));
    let err =
        stderr(&cli(&["run", "--cmd", "text.insert={\"text\":\"Hi\"}", "--cmd", "picture.size={\"width\":50}", "--save", &out.to_string_lossy()]));
    assert!(err.starts_with("wordcraft-cli: --cmd 2 of 2: `picture.size` is not available right now: select a picture or shape first."), "{err}");
    assert!(err.ends_with(" Nothing was saved.\n"), "{err}");
    assert!(!out.exists());
}

#[test]
fn print_shows_what_each_command_did() {
    assert_eq!(
        stdout(&cli(&[
            "run",
            "--print",
            "--cmd",
            "text.insert={\"text\":\"Bondata Oy\"}",
            "--cmd",
            "select.paragraph",
            "--cmd",
            "format.size={\"size\":28}",
            "--cmd",
            "insert.header={\"text\":\"Bondata\"}",
        ])),
        "text.insert → inserted `Bondata Oy` into paragraph 0, offsets 0–10; the caret is at paragraph 0, offset 10\n\
         select.paragraph → selected `Bondata Oy` in paragraph 0, offsets 0–10\n\
         format.size → formatted paragraph 0: size 28\n\
         insert.header → {\"changed\":\"added header 1 reading `Bondata`; the caret is at paragraph 0 of header 1, offset 0\",\"story\":1}\n\
         OK: 4 commands ran\n"
    );
}

#[test]
fn without_print_run_confirms_what_it_did() {
    let out = std::env::temp_dir().join(format!("wc-cli-ok-{}.docx", std::process::id()));
    let path = out.to_string_lossy().into_owned();
    assert_eq!(
        stdout(&cli(&["run", "--cmd", "text.insert={\"text\":\"Hi\"}", "--cmd", "format.bold", "--save", &path])),
        format!("OK: 2 commands ran, saved {path}\n")
    );
    assert!(out.exists());
    let _ = std::fs::remove_file(&out);
}

#[test]
fn options_and_values_are_checked_before_anything_runs() {
    assert_eq!(
        stderr(&cli(&["run", "--cmd", "text.insert", "--out", "x.docx"])),
        "wordcraft-cli: run was given `--out`, which it doesn't take. run takes --file, --template, --cmd, --save, --page, --scale, --print or --author; `wordcraft-cli help run` explains them.\n"
    );
    assert_eq!(
        stderr(&cli(&["run", "--save", "--print"])),
        "wordcraft-cli: run was given `--save` without a value; `wordcraft-cli help run` says what it takes.\n"
    );
    assert_eq!(
        stderr(&cli(&["run", "--template", "nope"])),
        "wordcraft-cli: `nope` is not a template: --template takes blank, sample, letter, resume or report.\n"
    );
    assert_eq!(
        stderr(&cli(&["render", "x.docx", "x.png", "--page", "two"])),
        "wordcraft-cli: --page takes a page number, 1 = the first; `two` is not one\n"
    );
    assert_eq!(
        stderr(&cli(&["run", "--page", "two", "--cmd", "format.bold"])),
        "wordcraft-cli: --page takes a page number, 1 = the first; `two` is not one\n"
    );
    assert_eq!(
        stderr(&cli(&["convert", "x.docx"])),
        "wordcraft-cli: convert is missing an argument: usage: wordcraft-cli convert IN OUT [--markup]\n"
    );
    assert_eq!(stderr(&cli(&["info", "nope.docx"])), "wordcraft-cli: can't open nope.docx: No such file or directory (os error 2)\n");
}
