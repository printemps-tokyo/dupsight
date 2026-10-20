use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use dupsight::group::{best_match, groups, within, Thresholds};
use dupsight::report;
use dupsight::scan;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Level {
    Strict,
    Normal,
    Loose,
}

#[derive(Parser, Debug)]
#[command(
    name = "dupsight",
    version,
    about = "Find near-duplicate images: resized, recompressed or lightly edited copies.",
    long_about = "Find near-duplicate images: resized, recompressed or lightly edited copies, grouped by \
                  perceptual hashes (dHash and pHash), with the file to keep suggested.\n\
                  dupsight only reports; it never deletes, moves or changes a file.",
    after_help = "Exit status: 0 done (scan) or the same picture (compare); 1 groups found with \
                  --check, or different pictures (compare); 2 an error."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Find groups of near-duplicate images in files and folders
    Scan(ScanArgs),
    /// Compare two images and show how alike they are (exit 0 the same picture, 1 different)
    Compare(CompareArgs),
}

#[derive(clap::Args, Debug)]
struct Match {
    /// How alike images must be: strict (near-identical copies), normal
    /// (resized, recompressed, lightly edited), loose (heavier edits)
    #[arg(long, value_enum, default_value = "normal")]
    level: Level,

    /// Largest dHash distance (0-64) still the same picture; overrides --level
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(0..=64))]
    max_dhash: Option<u32>,

    /// Largest pHash distance (0-64) still the same picture; overrides --level
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(0..=64))]
    max_phash: Option<u32>,

    /// Also match rotated (90, 180, 270 degrees) and mirrored copies
    #[arg(long)]
    rotations: bool,
}

impl Match {
    fn thresholds(&self) -> Thresholds {
        let name = match self.level {
            Level::Strict => "strict",
            Level::Normal => "normal",
            Level::Loose => "loose",
        };
        let base = Thresholds::preset(name).expect("preset");
        Thresholds {
            dhash: self.max_dhash.unwrap_or(base.dhash),
            phash: self.max_phash.unwrap_or(base.phash),
        }
    }
}

#[derive(clap::Args, Debug)]
struct ScanArgs {
    /// Files and folders to scan (folders are walked)
    #[arg(required = true, value_name = "PATH")]
    paths: Vec<PathBuf>,

    #[command(flatten)]
    matching: Match,

    /// Include hidden files and folders
    #[arg(long)]
    hidden: bool,

    /// JSON instead of text
    #[arg(long, conflicts_with = "extras")]
    json: bool,

    /// Print only the files other than the one to keep in each group, one per line
    #[arg(long)]
    extras: bool,

    /// With --extras, end each path with a NUL byte instead of a newline (for xargs -0)
    #[arg(short = '0', long = "null", requires = "extras")]
    null: bool,

    /// Also write a page with each group's images side by side
    #[arg(long, value_name = "FILE")]
    html: Option<PathBuf>,

    /// Exit 1 when any group is found (for CI)
    #[arg(long)]
    check: bool,

    /// Images decoded at the same time (default: one per CPU); fewer use less memory
    #[arg(short, long, value_name = "N", value_parser = clap::value_parser!(u16).range(1..))]
    jobs: Option<u16>,
}

#[derive(clap::Args, Debug)]
struct CompareArgs {
    /// The first image
    a: PathBuf,
    /// The second image (the one turned or mirrored with --rotations)
    b: PathBuf,

    #[command(flatten)]
    matching: Match,
}

/// The HTML report may only create a new file or replace an earlier report:
/// never a picture, a link, or any other file.
fn check_report_path(path: &Path) -> Result<()> {
    if scan::is_image(path) {
        bail!(
            "{}: the report needs a name that is not an image's (such as report.html)",
            report::shown(path)
        );
    }
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => bail!("{}: {e}", report::shown(path)),
    };
    let mut head = vec![0u8; report::HTML_MARK.len()];
    let is_report = meta.file_type().is_file()
        && std::fs::File::open(path)
            .and_then(|mut f| f.read_exact(&mut head))
            .is_ok()
        && head == report::HTML_MARK.as_bytes();
    if !is_report {
        bail!(
            "{}: exists and is not an earlier dupsight report; choose another name",
            report::shown(path)
        );
    }
    Ok(())
}

/// Write to a new file next to `path`, then rename it into place, so the
/// directory entry is replaced and a link's target is never written through.
fn write_report(path: &Path, page: &str) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.dupsight-{}.tmp", std::process::id()));
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| f.write_all(page.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn run(cli: Cli) -> Result<ExitCode> {
    let mut out = std::io::stdout().lock();
    match cli.command {
        Command::Scan(a) => {
            let t = a.matching.thresholds();
            if let Some(n) = a.jobs {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n as usize)
                    .build_global()
                    .context("cannot start the worker threads")?;
            }
            let (files, mut skipped) =
                scan::find(&a.paths, a.hidden).map_err(anyhow::Error::msg)?;
            if let Some(path) = &a.html {
                check_report_path(path)?;
            }
            let (images, unread) = scan::scan(&files, a.matching.rotations);
            skipped.extend(unread);
            let found = groups(&images, t);
            if let Some(path) = &a.html {
                write_report(path, &report::html(&images, &found))
                    .with_context(|| format!("cannot write {}", report::shown(path)))?;
            }
            if a.extras {
                let end = if a.null { "\0" } else { "\n" };
                for p in report::extras(&images, &found) {
                    out.write_all(&report::path_bytes(p))?;
                    out.write_all(end.as_bytes())?;
                }
                // The list has no room for them; tell the person instead.
                for k in &skipped {
                    eprintln!(
                        "dupsight: skipped {}: {}",
                        report::shown(&k.path),
                        report::clean(&k.reason)
                    );
                }
            } else if a.json {
                out.write_all(report::json(&images, &found, &skipped, t).as_bytes())?;
            } else {
                out.write_all(report::text(&images, &found, &skipped, t).as_bytes())?;
            }
            out.flush()?;
            if images.is_empty() && !skipped.is_empty() {
                eprintln!("dupsight: none of the files could be read");
                return Ok(ExitCode::from(2));
            }
            Ok(if a.check && !found.is_empty() {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            })
        }
        Command::Compare(c) => {
            let t = c.matching.thresholds();
            let load = |p: &PathBuf| {
                scan::load(p, false).map_err(|e| anyhow::anyhow!("{}: {e}", report::shown(p)))
            };
            let a = load(&c.a)?;
            let b = scan::load(&c.b, c.matching.rotations)
                .map_err(|e| anyhow::anyhow!("{}: {e}", report::shown(&c.b)))?;
            if scan::file_key(&a.path).is_some()
                && scan::file_key(&a.path) == scan::file_key(&b.path)
            {
                bail!("the two paths are the same file");
            }
            let m = best_match(&a, &b, t);
            let same = within(&m, t);
            writeln!(out, "dHash distance {} of 64 (limit {})", m.dhash, t.dhash)?;
            writeln!(out, "pHash distance {} of 64 (limit {})", m.phash, t.phash)?;
            if m.orientation > 0 {
                writeln!(out, "closest {}", report::orientation(m.orientation))?;
            }
            writeln!(
                out,
                "{}",
                if same {
                    "the same picture"
                } else {
                    "different pictures"
                }
            )?;
            Ok(if same {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("dupsight: {}", report::clean(&format!("{e:#}")));
            ExitCode::from(2)
        }
    }
}
