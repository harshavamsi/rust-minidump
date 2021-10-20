// Copyright 2015 Ted Mielczarek. See the COPYRIGHT
// file at the top-level directory of this distribution.

use std::boxed::Box;
use std::env;
use std::ops::Deref;
use std::panic;
use std::path::Path;
use std::path::PathBuf;

use minidump::*;
use minidump_processor::{
    http_symbol_supplier, simple_symbol_supplier, DwarfSymbolizer, MultiSymbolProvider, Symbolizer,
};

use clap::{crate_version, App, AppSettings, Arg};
use log::error;
use simplelog::{ColorChoice, ConfigBuilder, Level, LevelFilter, TermLogger, TerminalMode};

fn print_minidump_process(
    path: &Path,
    symbol_paths: Vec<PathBuf>,
    symbol_urls: Vec<String>,
    symbols_cache: PathBuf,
    symbols_tmp: PathBuf,
    human: bool,
    pretty: bool,
) {
    if let Ok(dump) = Minidump::read_path(path) {
        let mut provider = MultiSymbolProvider::new();

        if !symbol_urls.is_empty() {
            provider.add(Box::new(Symbolizer::new(http_symbol_supplier(
                symbol_paths,
                symbol_urls,
                symbols_cache,
                symbols_tmp,
            ))));
        } else if !symbol_paths.is_empty() {
            provider.add(Box::new(Symbolizer::new(simple_symbol_supplier(
                symbol_paths,
            ))));
        }
        provider.add(Box::new(DwarfSymbolizer::new()));

        match minidump_processor::process_minidump(&dump, &provider) {
            Ok(state) => {
                let mut stdout = std::io::stdout();
                if human {
                    state.print(&mut stdout).unwrap();
                } else {
                    state.print_json(&mut stdout, pretty).unwrap();
                }
            }
            Err(err) => {
                eprintln!("Error processing dump: {:?}", err);
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("Error reading dump");
        std::process::exit(1);
    }
}

#[cfg_attr(test, allow(dead_code))]
fn main() {
    let matches = App::new("minidump_stackwalk")
        .version(crate_version!())
        .about("Analyzes minidumps and produces a machine-readable JSON report.")
        .setting(AppSettings::NextLineHelp)
        .setting(AppSettings::ColoredHelp)
        .arg(
            Arg::with_name("human")
                .long_help("Emit a human-readable report instead.

The human-readable report does not have a specified format, and may not have as \
many details as the default JSON format. It is intended for quickly inspecting \
a crash or debugging rust-minidump itself.\n\n\n")
                .long("human")
        )
        .arg(
            Arg::with_name("pretty")
                .help("Pretty-print JSON output.")
                .long("pretty")
        )
        .arg(
            Arg::with_name("verbose")
                .long_help("Set the level of verbosity (off, error (default), warn, info, debug, trace)

The unwinder has been heavily instrumented with `trace` logging, so if you want to debug why \
an unwind happened the way it did, --verbose=trace is very useful (all unwinder logging will \
be prefixed with `unwind:`).\n\n\n")
                .long("verbose")
                .default_value("error")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("raw-json")
                .long_help("An input JSON file with the extra information (not yet implemented).

This is a gross hack for some legacy side-channel information that mozilla uses. It will \
hopefully be phased out and deprecated in favour of just using custom streams in the \
minidump itself.\n\n\n")
                .long("raw-json")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("symbols-url")
                .long_help("base URL from which URLs to symbol files can be constructed.

If multiple symbols-url values are provided, they will each be tried in order until \
one resolves.

The server the base URL points to is expected to conform to the Tecken \
symbol server protocol. For more details, see the Tecken docs:

https://tecken.readthedocs.io/en/latest/

Example symbols-url value: https://symbols.mozilla.org/\n\n\n")
                .multiple(true)
                .long("symbols-url")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("symbols-cache")
                .long_help("A directory in which downloaded symbols can be stored.
                    
Symbol files can be very large, so we recommend placing cached files in your \
system's temp directory so that it can garbage collect unused ones for you. \
To this end, the default value for this flag is a `rust-minidump-cache` \
subdirectory of `std::env::temp_dir()` (usually /tmp/rust-minidump-cache on linux).

symbols-cache must be on the same filesystem as symbols-tmp (if that doesn't mean anything to \
you, don't worry about it, you're probably not doing something that will run afoul of it).
\n\n\n")
                .long("symbols-cache")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("symbols-tmp")
                .long_help("A directory to use as temp space for downloading symbols.

A temp dir is necessary to allow for multiple rust-minidump instances to share a cache without \
race conditions. Files to be added to the cache will be constructed in this location before \
being atomically moved to the cache.

If no path is specified, `std::env::temp_dir()` will be used to improve portability. \
See the rust documentation for how to set that value if you wish to use something other than \
your system's default temp directory.

symbols-tmp must be on the same filesystem as symbols-cache (if that doesn't mean anything to \
you, don't worry about it, you're probably not doing something that will run afoul of it).
\n\n\n")
                .long("symbols-tmp")
                .takes_value(true),
        )
        .arg(
            Arg::with_name("minidump")
                .help("Path to the minidump file to analyze.")
                .required(true)
                .takes_value(true)
        )
        .arg(
            Arg::with_name("symbols-path")
                .long_help("Path to a symbol file.

If multiple symbols-path values are provided, all symbol files will be merged \
into minidump-stackwalk's symbol database.\n\n\n")
                .multiple(true)
                .takes_value(true)
        )
        .after_help("

Purpose of Symbols:

  Symbols are used for two purposes:

  1. To fill in more information about each frame of the backtraces. (function names, lines, etc.)

  2. To do produce a more *accurate* backtrace. This is primarily accomplished with \
call frame information (CFI), but just knowing what parts of a module maps to actual \
code is also useful!



Supported Symbol Formats:

  Currently only breakpad text symbol files are supported, although we hope to eventually \
support native formats like PDB and DWARF as well.



Breakpad Symbol Files:

  Breakpad symbol files are basically a simplified version of the information found in \
native debuginfo formats. We recommend using a version of dump_syms to generate them.

  See:
    * symbol file docs: https://chromium.googlesource.com/breakpad/breakpad/+/master/docs/symbol_files.md
    * mozilla's dump_syms (co-developed with this program): https://github.com/mozilla/dump_syms

")
        .get_matches();

    let verbosity = match matches.value_of("verbose").unwrap() {
        "off" => LevelFilter::Off,
        "warn" => LevelFilter::Warn,
        "info" => LevelFilter::Info,
        "debug" => LevelFilter::Debug,
        "trace" => LevelFilter::Trace,
        _ => LevelFilter::Error,
    };

    // Init the logger (and make trace logging less noisy)
    let _ = TermLogger::init(
        verbosity,
        ConfigBuilder::new()
            .set_location_level(LevelFilter::Off)
            .set_time_level(LevelFilter::Off)
            .set_thread_level(LevelFilter::Off)
            .set_target_level(LevelFilter::Off)
            .set_level_color(Level::Trace, None)
            .build(),
        TerminalMode::Stderr,
        ColorChoice::Auto,
    );

    // Set a panic hook to redirect to the logger
    panic::set_hook(Box::new(|panic_info| {
        let (filename, line) = panic_info
            .location()
            .map(|loc| (loc.file(), loc.line()))
            .unwrap_or(("<unknown>", 0));
        let cause = panic_info
            .payload()
            .downcast_ref::<String>()
            .map(String::deref)
            .unwrap_or_else(|| {
                panic_info
                    .payload()
                    .downcast_ref::<&str>()
                    .copied()
                    .unwrap_or("<cause unknown>")
            });
        error!("A panic occurred at {}:{}: {}", filename, line, cause);
    }));

    // All options the original minidump-stackwalk has, stubbed out for when we need them:
    let _json_path = matches.value_of_os("raw-json").map(Path::new);

    let temp_dir = std::env::temp_dir();

    // Default to env::temp_dir()/rust-minidump-cache
    let symbols_cache = matches
        .value_of_os("symbols-cache")
        .map(|os_str| Path::new(os_str).to_owned())
        .unwrap_or_else(|| temp_dir.join("rust-minidump-cache"));

    // Default to env::temp_dir()
    let symbols_tmp = matches
        .value_of_os("symbols-tmp")
        .map(|os_str| Path::new(os_str).to_owned())
        .unwrap_or(temp_dir);

    let symbols_urls = matches
        .values_of("symbols-url")
        .map(|v| v.map(String::from).collect::<Vec<_>>())
        .unwrap_or_else(Vec::new);

    let pretty = matches.is_present("pretty");
    let human = matches.is_present("human");

    let minidump_path = matches.value_of_os("minidump").map(Path::new).unwrap();
    let symbols_paths = matches
        .values_of_os("symbols-path")
        .map(|v| {
            v.map(|os_str| Path::new(os_str).to_owned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(Vec::new);

    if pretty && human {
        eprintln!("Humans must be hideous! (The --pretty and --human flags cannot both be set)");
        std::process::exit(1);
    }

    print_minidump_process(
        minidump_path,
        symbols_paths,
        symbols_urls,
        symbols_cache,
        symbols_tmp,
        human,
        pretty,
    );
}
