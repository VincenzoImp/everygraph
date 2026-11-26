// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Reading the command line, by hand (`design/DECISIONS.md`, "Reading the command
//! line"): options as `--name VALUE` or `--name=VALUE`, `--` before arguments that
//! start with `-`, and the options that mean the same on every command
//! (`docs/api-sketch.md` § 5.2).

use std::ffi::{OsStr, OsString};
use std::num::NonZeroUsize;
use std::path::PathBuf;

use crate::failure::{Failure, usage};
use crate::render::parse_size;

/// One argument.
pub(crate) enum Arg<'a> {
    /// An option, such as `--memory`; its value, if it takes one, comes from
    /// [`Args::value`].
    Option(&'a str),
    /// Anything else.
    Plain(&'a OsStr),
}

/// The arguments after the command's name.
pub(crate) struct Args<'a> {
    rest: std::slice::Iter<'a, OsString>,
    command: &'static str,
    /// The value given as `--name=VALUE`, not yet taken.
    attached: Option<(&'a str, &'a str)>,
    only_plain: bool,
}

impl<'a> Args<'a> {
    pub(crate) fn new(args: &'a [OsString], command: &'static str) -> Self {
        Self {
            rest: args.iter(),
            command,
            attached: None,
            only_plain: false,
        }
    }

    /// A usage error of this command, with its help as the way forward.
    pub(crate) fn usage(&self, what: String) -> Failure {
        usage(what, self.command)
    }

    /// The next argument, or `None` at the end.
    ///
    /// # Errors
    ///
    /// A value attached to the previous option, which takes none; an option that is not
    /// valid text.
    pub(crate) fn next(&mut self) -> Result<Option<Arg<'a>>, Failure> {
        if let Some((name, _)) = self.attached.take() {
            return Err(self.usage(format!("{name} takes no value.")));
        }
        let Some(arg) = self.rest.next() else {
            return Ok(None);
        };
        if self.only_plain {
            return Ok(Some(Arg::Plain(arg)));
        }
        let Some(text) = arg.to_str().filter(|t| t.starts_with('-') && t.len() > 1) else {
            return Ok(Some(Arg::Plain(arg)));
        };
        if text == "--" {
            self.only_plain = true;
            return self.next();
        }
        match text.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                self.attached = Some((name, value));
                Ok(Some(Arg::Option(name)))
            }
            _ => Ok(Some(Arg::Option(text))),
        }
    }

    /// The value of option `name`.
    ///
    /// # Errors
    ///
    /// There is none, or it is not valid text.
    pub(crate) fn value(&mut self, name: &str) -> Result<&'a str, Failure> {
        if let Some((_, value)) = self.attached.take() {
            return Ok(value);
        }
        self.rest
            .next()
            .and_then(|v| v.to_str())
            .ok_or_else(|| self.usage(format!("{name} needs a value.")))
    }

    /// The value of `name`, one of `choices`.
    pub(crate) fn choice<T: Copy>(
        &mut self,
        name: &str,
        choices: &[(&str, T)],
    ) -> Result<T, Failure> {
        let value = self.value(name)?;
        choices
            .iter()
            .find(|(word, _)| *word == value)
            .map(|&(_, v)| v)
            .ok_or_else(|| {
                let words: Vec<&str> = choices.iter().map(|(w, _)| *w).collect();
                self.usage(format!("{name} takes {}, not {value}.", words.join(" or ")))
            })
    }

    /// The value of `name`, parsed.
    pub(crate) fn parsed<T: std::str::FromStr>(
        &mut self,
        name: &str,
        what: &str,
    ) -> Result<T, Failure> {
        let value = self.value(name)?;
        value
            .parse()
            .map_err(|_| self.usage(format!("{name} {value} is not {what}.")))
    }

    /// The error for an option this command does not have.
    pub(crate) fn unknown(&self, option: &str) -> Failure {
        self.usage(format!("Unknown option {option}."))
    }
}

/// The options shared by the commands that compute (`docs/api-sketch.md` § 5.2).
#[derive(Debug, Default)]
pub(crate) struct Common {
    /// `--memory`: the bytes, and the text as typed.
    pub(crate) memory: Option<(u64, String)>,
    pub(crate) threads: Option<NonZeroUsize>,
    pub(crate) json: bool,
    pub(crate) out: Option<PathBuf>,
    pub(crate) overwrite: bool,
    /// `--progress=plain` (S-10).
    pub(crate) plain: bool,
}

impl Common {
    /// Takes `option` if it is one of these; returns whether it was.
    pub(crate) fn take(&mut self, option: &str, args: &mut Args<'_>) -> Result<bool, Failure> {
        match option {
            "--memory" => {
                let text = args.value(option)?;
                let bytes = parse_size(text).ok_or_else(|| {
                    args.usage(format!("--memory {text} is not a size, such as 8GB."))
                })?;
                self.memory = Some((bytes, text.to_owned()));
            }
            "--threads" => self.threads = Some(args.parsed(option, "a positive number")?),
            "--json" => self.json = true,
            "--out" | "-o" => self.out = Some(PathBuf::from(args.value(option)?)),
            "--overwrite" => self.overwrite = true,
            "--progress" => {
                self.plain = args.choice(option, &[("plain", true), ("auto", false)])?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }
}

/// `-h` or `--help`.
pub(crate) fn is_help(option: &str) -> bool {
    option == "-h" || option == "--help"
}
