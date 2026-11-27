#![no_main]

mod sandbox;

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    sandbox::enable("format_expand");

    tmux_rs_new::format::fuzz_format_expand(data);
});
