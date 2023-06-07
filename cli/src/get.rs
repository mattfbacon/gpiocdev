// SPDX-FileCopyrightText: 2021 Kent Gibson <warthog618@gmail.com>
//
// SPDX-License-Identifier: Apache-2.0 OR MIT

use super::common::{self, emit_error};
use anyhow::{Context, Result};
use clap::Parser;
use gpiocdev::line::{Offset, Value, Values};
use gpiocdev::request::{Config, Request};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(alias("g"))]
pub struct Opts {
    /// The lines to get
    ///
    /// The lines are identified by name or optionally by offset
    /// if the --chip option is provided.
    #[arg(value_name = "line", required = true)]
    line: Vec<String>,

    #[command(flatten)]
    line_opts: common::LineOpts,

    /// Request the line as-is rather than as an input
    ///
    /// If not specified then the lines are explicitly switched to being input lines.
    ///
    /// If specified then the line direction is left as-is, making it
    /// possible to read back the values of output lines.
    #[arg(short, long)]
    as_is: bool,

    #[command(flatten)]
    active_low_opts: common::ActiveLowOpts,

    #[command(flatten)]
    bias_opts: common::BiasOpts,

    /// Wait between requesting the lines and reading the values
    ///
    /// This provides time for any bias setting to take effect.
    ///
    /// The period is taken as milliseconds unless otherwise specified.
    #[arg(short = 'p', long, value_name = "period", value_parser = common::parse_duration)]
    hold_period: Option<Duration>,

    /// Display line values as '0' (inactive) or '1' (active)
    #[arg(long)]
    pub numeric: bool,

    #[command(flatten)]
    uapi_opts: common::UapiOpts,

    /// The consumer label applied to requested lines.
    #[arg(short = 'C', long, value_name = "name", default_value = "gpiocdev-get")]
    consumer: String,

    #[command(flatten)]
    emit: common::EmitOpts,
}

impl Opts {
    // mutate the config to match the configuration
    fn apply(&self, config: &mut Config) {
        self.active_low_opts.apply(config);
        self.bias_opts.apply(config);
        if !self.as_is {
            config.as_input();
        }
    }
}

pub fn cmd(opts: &Opts) -> bool {
    match cmd_inner(opts) {
        Err(e) => {
            emit_error(&opts.emit, &e);
            false
        }
        Ok(x) => x,
    }
}

fn cmd_inner(opts: &Opts) -> Result<bool> {
    let abiv = common::actual_abi_version(&opts.uapi_opts)?;
    let r = common::Resolver::resolve_lines(&opts.line, &opts.line_opts, abiv)?;
    let mut requests = Vec::new();
    for (idx, ci) in r.chips.iter().enumerate() {
        let mut cfg = Config::default();
        opts.apply(&mut cfg);
        let offsets: Vec<Offset> = r
            .lines
            .values()
            .filter(|co| co.chip_idx == idx)
            .map(|co| co.offset)
            .collect();
        cfg.with_lines(&offsets);

        let mut bld = Request::from_config(cfg);
        bld.on_chip(&ci.path).with_consumer(&opts.consumer);
        #[cfg(all(feature = "uapi_v1", feature = "uapi_v2"))]
        bld.using_abi_version(abiv);
        let req = bld
            .request()
            .with_context(|| format!("failed to request lines {:?} from {}", offsets, ci.name))?;
        requests.push(req);
    }
    if let Some(period) = opts.hold_period {
        thread::sleep(period);
    }
    let mut line_values: HashMap<String, Value> = HashMap::new();
    for (idx, ci) in r.chips.iter().enumerate() {
        let mut values = Values::default();
        requests[idx]
            .values(&mut values)
            .with_context(|| format!("failed to read values from {}", ci.name))?;
        for line in r.lines.iter().filter(|l| l.1.chip_idx == idx) {
            line_values.insert(line.0.into(), values.get(line.1.offset).unwrap());
        }
    }
    emit_values(opts, &line_values);

    Ok(true)
}

fn emit_values(opts: &Opts, values: &HashMap<String, Value>) {
    print_values(opts, values);
}

fn print_values(opts: &Opts, values: &HashMap<String, Value>) {
    let mut print_values = Vec::new();
    for id in &opts.line {
        let value = values.get(id).unwrap();
        print_values.push(if opts.numeric {
            let v: u8 = (*value).into();
            format!("{}", v)
        } else if opts.emit.quoted {
            format!("\"{}\"={}", id, value)
        } else {
            format!("{}={}", id, value)
        })
    }
    println!("{}", print_values.join(" "));
}
