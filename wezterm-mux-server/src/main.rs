use config::configuration;
use mux::activity::Activity;
use mux::domain::{Domain, LocalDomain};
use mux::Mux;
use portable_pty::cmdbuilder::CommandBuilder;
use std::ffi::OsString;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;
use std::thread;
use structopt::*;
use wezterm_gui_subcommands::*;

mod daemonize;

#[derive(Debug, StructOpt)]
#[structopt(
    about = "Wez's Terminal Emulator\nhttp://github.com/wez/wezterm",
    global_setting = structopt::clap::AppSettings::ColoredHelp,
    version = config::wezterm_version()
)]
struct Opt {
    /// Skip loading wezterm.lua
    #[structopt(name = "skip-config", short = "n")]
    skip_config: bool,

    /// Specify the configuration file to use, overrides the normal
    /// configuration file resolution
    #[structopt(
        long = "config-file",
        parse(from_os_str),
        conflicts_with = "skip-config"
    )]
    config_file: Option<OsString>,

    /// Override specific configuration values
    #[structopt(
        long = "config",
        name = "name=value",
        parse(try_from_str = name_equals_value),
        number_of_values = 1)]
    config_override: Vec<(String, String)>,

    /// Detach from the foreground and become a background process
    #[structopt(long = "daemonize")]
    daemonize: bool,

    /// Specify the current working directory for the initially
    /// spawned program
    #[structopt(long = "cwd", parse(from_os_str))]
    cwd: Option<OsString>,

    /// Instead of executing your shell, run PROG.
    /// For example: `wezterm start -- bash -l` will spawn bash
    /// as if it were a login shell.
    #[structopt(parse(from_os_str))]
    prog: Vec<OsString>,
}

fn main() {
    if let Err(err) = run() {
        log::error!("{:#}", err);
        std::process::exit(1);
    }
}

fn run() -> anyhow::Result<()> {
    env_bootstrap::bootstrap();

    //stats::Stats::init()?;
    config::designate_this_as_the_main_thread();
    let _saver = umask::UmaskSaver::new();

    let opts = Opt::from_args();
    config::common_init(
        opts.config_file.as_ref(),
        &opts.config_override,
        opts.skip_config,
    );

    let config = config::configuration();
    #[cfg(unix)]
    {
        if opts.daemonize {
            daemonize::daemonize(&config)?;
            // When we reach this line, we are in a forked child process,
            // and the fork will have broken the async-io/reactor state
            // of the smol runtime.
            // To resolve this, we will re-exec ourselves in the block
            // below that was originally Windows-specific
        }
    }

    if opts.daemonize {
        // On Windows we can't literally daemonize, but we can spawn another copy
        // of ourselves in the background!
        // On Unix, forking breaks the global state maintained by `smol`,
        // so we need to re-exec ourselves to start things back up properly.
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        if opts.skip_config {
            cmd.arg("-n");
        }
        if let Some(cwd) = opts.cwd {
            cmd.arg("--cwd");
            cmd.arg(cwd);
        }
        if !opts.prog.is_empty() {
            cmd.arg("--");
            for a in &opts.prog {
                cmd.arg(a);
            }
        }

        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.stdout(config.daemon_options.open_stdout()?);
            cmd.stderr(config.daemon_options.open_stderr()?);

            cmd.creation_flags(winapi::um::winbase::DETACHED_PROCESS);
            let child = cmd.spawn();
            drop(child);
            return Ok(());
        }

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            if let Some(mask) = umask::UmaskSaver::saved_umask() {
                unsafe {
                    cmd.pre_exec(move || {
                        libc::umask(mask);
                        Ok(())
                    });
                }
            }

            return Err(anyhow::anyhow!("failed to re-exec: {:?}", cmd.exec()));
        }
    }

    // Remove some environment variables that aren't super helpful or
    // that are potentially misleading when we're starting up the
    // server.
    // We may potentially want to look into starting/registering
    // a session of some kind here as well in the future.
    for name in &[
        "OLDPWD",
        "PWD",
        "SHLVL",
        "WEZTERM_PANE",
        "WEZTERM_UNIX_SOCKET",
        "_",
    ] {
        std::env::remove_var(name);
    }
    for name in &config::configuration().mux_env_remove {
        std::env::remove_var(name);
    }

    let need_builder = !opts.prog.is_empty() || opts.cwd.is_some();

    let cmd = if need_builder {
        let mut builder = if opts.prog.is_empty() {
            CommandBuilder::new_default_prog()
        } else {
            CommandBuilder::from_argv(opts.prog)
        };
        if let Some(cwd) = opts.cwd {
            builder.cwd(cwd);
        }
        Some(builder)
    } else {
        None
    };

    let domain: Arc<dyn Domain> = Arc::new(LocalDomain::new("local")?);
    let mux = Rc::new(mux::Mux::new(Some(domain.clone())));
    Mux::set_mux(&mux);

    let executor = promise::spawn::SimpleExecutor::new();

    spawn_listener().map_err(|e| {
        log::error!("problem spawning listeners: {:?}", e);
        e
    })?;

    let activity = Activity::new();

    promise::spawn::spawn(async move {
        if let Err(err) = async_run(cmd).await {
            terminate_with_error(err);
        }
        drop(activity);
    })
    .detach();

    loop {
        executor.tick()?;
    }
}

async fn async_run(cmd: Option<CommandBuilder>) -> anyhow::Result<()> {
    let mux = Mux::get().unwrap();

    let domain = mux.default_domain();
    let window_id = mux.new_empty_window(None);
    domain.attach(Some(*window_id)).await?;

    let config = config::configuration();
    let _tab = mux
        .default_domain()
        .spawn(config.initial_size(), cmd, None, *window_id)
        .await?;
    Ok(())
}

fn terminate_with_error(err: anyhow::Error) -> ! {
    log::error!("{:#}; terminating", err);
    std::process::exit(1);
}

mod ossl;

pub fn spawn_listener() -> anyhow::Result<()> {
    let config = configuration();
    for unix_dom in &config.unix_domains {
        std::env::set_var("WEZTERM_UNIX_SOCKET", unix_dom.socket_path());
        let mut listener = wezterm_mux_server_impl::local::LocalListener::with_domain(unix_dom)?;
        thread::spawn(move || {
            listener.run();
        });
    }

    for tls_server in &config.tls_servers {
        ossl::spawn_tls_listener(tls_server)?;
    }

    Ok(())
}
