use crate::{
    config_gen,
    plugin::{Map as PluginMap, RunError as PluginError},
    steps::{Registry as StepRegistry, StepNotRegistered, Steps},
};
use ginit_core::{
    config::umbrella::{self, Umbrella},
    opts,
    util::{self, cli},
};
use std::fmt::{self, Display};
use structopt::clap::{App, Arg, ArgMatches, SubCommand};

fn take_a_list<'a, 'b>(arg: Arg<'a, 'b>, values: &'a [&'a str]) -> Arg<'a, 'b> {
    arg.possible_values(values)
        .multiple(true)
        .value_delimiter(" ")
}

pub fn app<'a, 'b>(steps: &'a [&'a str]) -> App<'a, 'b> {
    SubCommand::with_name("init")
        .about("Creates a new project in the current working directory")
        .arg_from_usage("--force 'Clobber files with no remorse'")
        .arg_from_usage("--open 'Open in default code editor'")
        .arg(take_a_list(
            Arg::with_name("only")
                .long("only")
                .help("Only do some steps")
                .value_name("STEPS"),
            steps,
        ))
        .arg(take_a_list(
            Arg::with_name("skip")
                .long("skip")
                .help("Skip some steps")
                .value_name("STEPS"),
            steps,
        ))
}

#[derive(Debug)]
pub enum Error {
    OnlyParseFailed(StepNotRegistered),
    SkipParseFailed(StepNotRegistered),
    StepNotRegistered(StepNotRegistered),
    ConfigLoadFailed(umbrella::Error),
    ConfigGenFailed(config_gen::Error),
    PluginFailed {
        plugin_name: String,
        cause: PluginError,
    },
    OpenInEditorFailed(util::OpenInEditorError),
}

impl Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::OnlyParseFailed(err) => write!(f, "Failed to parse `only` step list: {}", err),
            Error::SkipParseFailed(err) => write!(f, "Failed to parse `skip` step list: {}", err),
            Error::StepNotRegistered(err) => write!(f, "{}", err),
            Error::ConfigLoadFailed(err) => write!(f, "{}", err),
            Error::ConfigGenFailed(err) => write!(f, "Failed to generate config: {}", err),
            Error::PluginFailed{
                plugin_name,
                cause,
            } => write!(f, "Failed to init {:?} plugin: {}", plugin_name, cause),
            Error::OpenInEditorFailed(err) => write!(f, "Failed to open project in editor (your project generated successfully though, so no worries!): {}", err),
        }
    }
}

#[derive(Debug)]
pub struct Command {
    clobbering: opts::Clobbering,
    open_in: opts::OpenIn,
    only: Option<Vec<String>>,
    skip: Option<Vec<String>>,
}

impl Command {
    pub fn parse(matches: &ArgMatches<'_>) -> Self {
        let clobbering = cli::clobbering_from_presence(matches.is_present("force"));
        let open_in = if matches.is_present("open") {
            opts::OpenIn::Editor
        } else {
            opts::OpenIn::Nothing
        };
        let only = matches.args.get("only").map(|only| {
            only.vals
                .iter()
                .map(|step| step.to_string_lossy().into_owned())
                .collect()
        });
        let skip = matches.args.get("skip").map(|skip| {
            skip.vals
                .iter()
                .map(|step| step.to_string_lossy().into_owned())
                .collect()
        });
        Self {
            clobbering,
            open_in,
            only,
            skip,
        }
    }
}

pub fn exec(
    noise_level: opts::NoiseLevel,
    interactivity: opts::Interactivity,
    Command {
        clobbering,
        open_in,
        only,
        skip,
    }: Command,
    plugins: &PluginMap,
    wrapper: &util::TextWrapper,
) -> Result<(), Error> {
    let only = only.as_ref().map(|only| only.as_slice());
    let skip = skip.as_ref().map(|skip| skip.as_slice());
    let step_registry = {
        let mut registry = StepRegistry::default();
        for plugin in plugins.iter() {
            registry.register(plugin.name());
        }
        registry
    };
    let steps = {
        let only = only
            .map(|only| Steps::parse(&step_registry, only))
            .unwrap_or_else(|| Ok(Steps::new_all_set(&step_registry)))
            .map_err(Error::OnlyParseFailed)?;
        let skip = skip
            .map(|skip| Steps::parse(&step_registry, skip))
            .unwrap_or_else(|| Ok(Steps::new_all_unset(&step_registry)))
            .map_err(Error::SkipParseFailed)?;
        Steps::from_bits(&step_registry, only.bits() & !skip.bits())
    };
    let args = {
        let mut args = vec!["init"];
        if let opts::Clobbering::Allow = clobbering {
            args.push("--force");
        }
        args
    };
    if let None = Umbrella::load(".").map_err(Error::ConfigLoadFailed)? {
        config_gen::gen_and_write(
            clobbering,
            noise_level,
            interactivity,
            ".",
            &plugins,
            wrapper,
        )
        .map_err(Error::ConfigGenFailed)?;
    }
    for plugin in plugins.iter() {
        if steps
            .is_set(plugin.name())
            .map_err(Error::StepNotRegistered)?
        {
            plugin
                .run_and_wait(noise_level, interactivity, &args)
                .map_err(|cause| Error::PluginFailed {
                    plugin_name: plugin.name().to_owned(),
                    cause,
                })?;
        }
    }
    if let opts::OpenIn::Editor = open_in {
        util::open_in_editor(".").map_err(Error::OpenInEditorFailed)?;
    }
    Ok(())
}
