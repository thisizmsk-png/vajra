import json
import unittest

from waiting_calls import ALLOWANCE, commands, count_core, count_ui, problems, read_repo

RUST = '''
#[tauri::command]
fn quick(app: State<'_, App>) -> Res { Ok(app.value()) }

#[tauri::command(async)]
fn off_main(app: State<'_, App>) -> Res { std::fs::read_to_string("x"); Ok(json!({})) }

#[tauri::command]
async fn also_off_main(h: AppHandle) -> Res { spawn_blocking(move || std::fs::read("x")).await }

// #[tauri::command]
// fn commented_out() -> Res { Ok(json!({})) }

/// Doc comment.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn reads_a_file(a: u8) -> Res {
    let s = std::fs::read_to_string("/tmp/x").unwrap(); // std::fs::read in a comment does not count
    std::thread::sleep(D);
    Ok(json!({"s": s}))
}
'''


class Counting(unittest.TestCase):
    def test_a_plain_fn_command_is_main_thread_and_async_ones_are_not(self):
        got = {n: main for n, main, _ in commands(RUST)}
        self.assertEqual(got, {"quick": True, "off_main": False, "also_off_main": False, "reads_a_file": True})

    def test_a_commented_out_command_is_not_counted(self):
        self.assertNotIn("commented_out", {n for n, _, _ in commands(RUST)})

    def test_blocking_calls_count_only_in_main_thread_bodies_and_not_in_comments(self):
        names, blocking, unparsed = count_core([RUST])
        self.assertEqual(unparsed, [])
        self.assertEqual(names, ["quick", "reads_a_file"])
        self.assertEqual(blocking, 2)  # the read_to_string and the sleep; the async command's read and the comment are not

    def test_a_command_with_restricted_visibility_is_still_counted(self):
        src = "\n".join([
            "#[tauri::command]", "pub(crate) fn slow_read() -> Res { std::fs::read(\"x\"); Ok(json!({})) }",
            "#[tauri::command]", "pub(super) fn b() -> Res { Ok(json!({})) }",
            "#[tauri::command]", "pub(in crate::x) fn c() -> Res { Ok(json!({})) }",
            "#[tauri::command(async)]", "pub(crate) fn off() -> Res { Ok(json!({})) }",
            "#[tauri::command]", "pub(crate) async fn also_off() -> Res { Ok(json!({})) }",
            "#[tauri::command]", "pub unsafe fn d() -> Res { Ok(json!({})) }",
        ])
        names, blocking, unparsed = count_core([src])
        self.assertEqual((names, blocking, unparsed), (["b", "c", "d", "slow_read"], 1, []))

    def test_a_command_attribute_the_scanner_cannot_parse_is_reported_not_skipped(self):
        names, _, unparsed = count_core(["#[tauri::command]\nmacro_generated_command!(slow_read);\n", "#[tauri::command]\n"])
        self.assertEqual(names, [])
        self.assertEqual(unparsed, ["macro_generated_command!(slow_read);", "<end of file>"])
        out = problems({**Ratchet.ALLOWED, "unparsed_commands": unparsed}, Ratchet.ALLOWED)
        self.assertEqual(len([l for l in out if "cannot read" in l]), 2, out)

    def test_ui_sync_calls(self):
        self.assertEqual(count_ui(["const x = new XMLHttpRequest(); // XMLHttpRequest again", "readFileSync(p); invokeSync(a)"]), 3)
        self.assertEqual(count_ui(["await invoke('x'); const syncing = 1;"]), 0)


class Ratchet(unittest.TestCase):
    ALLOWED = {"main_thread_commands": ["a", "b"], "blocking_calls_in_main_thread_commands": 1, "sync_ui_calls": 0}

    def test_equal_is_ok(self):
        self.assertEqual(problems(dict(self.ALLOWED), self.ALLOWED), [])

    def test_a_new_main_thread_command_fails(self):
        out = problems({**self.ALLOWED, "main_thread_commands": ["a", "b", "c"]}, self.ALLOWED)
        self.assertTrue(any("new main-thread command" in l and "'c'" in l for l in out), out)

    def test_swapping_one_for_another_fails_even_though_the_count_is_the_same(self):
        self.assertTrue(problems({**self.ALLOWED, "main_thread_commands": ["a", "c"]}, self.ALLOWED))

    def test_a_removed_command_must_come_off_the_allowance(self):
        out = problems({**self.ALLOWED, "main_thread_commands": ["a"]}, self.ALLOWED)
        self.assertTrue(any("are gone" in l for l in out), out)

    def test_more_blocking_calls_or_sync_ui_calls_fail(self):
        self.assertTrue(problems({**self.ALLOWED, "blocking_calls_in_main_thread_commands": 2}, self.ALLOWED))
        self.assertTrue(problems({**self.ALLOWED, "sync_ui_calls": 1}, self.ALLOWED))

    def test_fewer_fails_until_the_allowance_is_lowered(self):
        out = problems({**self.ALLOWED, "blocking_calls_in_main_thread_commands": 0}, self.ALLOWED)
        self.assertTrue(any("lower the allowance to 0" in l for l in out), out)


class ThisRepo(unittest.TestCase):
    def test_the_counts_equal_the_allowance(self):
        self.assertEqual(problems(read_repo(), json.loads(ALLOWANCE.read_text())), [])

    def test_the_real_terminal_write_is_still_found_so_the_scan_is_not_blind(self):
        self.assertIn("term_write", read_repo()["main_thread_commands"])


if __name__ == "__main__":
    unittest.main()
