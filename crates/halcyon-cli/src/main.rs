//! Development CLI for the Halcyon core.
//!
//! Lets us set up real accounts, sync and search before the desktop UI
//! exists. It uses the same database and keychain entries as the app will.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use chrono::{Local, TimeZone};
use clap::{Parser, Subcommand};
use halcyon_core::autoconfig::{self, Candidate};
use halcyon_core::imap::{self, Credentials};
use halcyon_core::model::{
    Account, AuthMethod, FolderRole, MessageSummary, Security, ServerConfig,
};
use halcyon_core::secrets::{self, SecretKind};
use halcyon_core::store::Store;
use halcyon_core::sync::{self, SyncEvent};

#[derive(Parser)]
#[command(name = "halcyon", version, about = "Halcyon mail – development CLI")]
struct Cli {
    /// Database file (default: ~/.local/share/halcyon/halcyon.db).
    #[arg(long, global = true, env = "HALCYON_DB")]
    db: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show which server settings would be used for an address.
    Discover { email: String },
    /// Manage accounts.
    #[command(subcommand)]
    Account(AccountCommand),
    /// Fetch new mail for all accounts, or only for one.
    Sync { email: Option<String> },
    /// Newest messages of all inboxes.
    Inbox {
        #[arg(short, long, default_value_t = 30)]
        limit: usize,
    },
    /// Search all accounts (subject, sender, recipients).
    Search {
        #[arg(required = true)]
        query: Vec<String>,
        #[arg(short, long, default_value_t = 30)]
        limit: usize,
    },
    /// Run local case/source analysis and print privacy-preserving numbers only.
    Stats,
}

#[derive(Subcommand)]
enum AccountCommand {
    /// Add an account. Settings are discovered automatically unless given.
    Add {
        email: String,
        #[arg(long)]
        name: Option<String>,
        /// IMAP server, skips discovery.
        #[arg(long)]
        imap_host: Option<String>,
        #[arg(long, requires = "imap_host")]
        imap_port: Option<u16>,
        /// tls, starttls or plain.
        #[arg(long, requires = "imap_host")]
        imap_security: Option<String>,
        /// Login name if it differs from the address.
        #[arg(long)]
        username: Option<String>,
    },
    List,
    Remove {
        email: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("HALCYON_LOG")
                // The platform verifier logs every failed autoconfig probe as an error.
                .unwrap_or_else(|_| "warn,rustls_platform_verifier=off".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let db_path = match cli.db {
        Some(p) => p,
        None => directories::ProjectDirs::from("", "", "halcyon")
            .context("no home directory")?
            .data_dir()
            .join("halcyon.db"),
    };

    match cli.command {
        Command::Discover { email } => discover(&email).await,
        Command::Account(cmd) => {
            let store = Store::open(&db_path)?;
            match cmd {
                AccountCommand::Add {
                    email,
                    name,
                    imap_host,
                    imap_port,
                    imap_security,
                    username,
                } => {
                    let manual = match imap_host {
                        Some(host) => {
                            let security: Security =
                                imap_security.as_deref().unwrap_or("tls").parse()?;
                            let port = imap_port.unwrap_or(match security {
                                Security::Tls => 993,
                                _ => 143,
                            });
                            Some(ServerConfig {
                                host,
                                port,
                                security,
                                username: username.clone().unwrap_or_else(|| email.clone()),
                                auth: AuthMethod::Password,
                            })
                        }
                        None => None,
                    };
                    add_account(&store, &email, name, manual, username).await
                }
                AccountCommand::List => {
                    for a in store.accounts()? {
                        println!(
                            "{:<32} {}:{} ({}, {})",
                            a.email, a.imap.host, a.imap.port, a.imap.security, a.imap.username
                        );
                    }
                    Ok(())
                }
                AccountCommand::Remove { email } => {
                    let account = store
                        .account_by_email(&email)?
                        .with_context(|| format!("no account {email}"))?;
                    store.remove_account(account.id)?;
                    secrets::delete(&account.email, SecretKind::ImapPassword)?;
                    println!("Removed {} and its local copy of the mail.", account.email);
                    Ok(())
                }
            }
        }
        Command::Sync { email } => {
            let mut store = Store::open(&db_path)?;
            run_sync(&mut store, email.as_deref()).await
        }
        Command::Inbox { limit } => {
            let store = Store::open(&db_path)?;
            print_messages(&store.recent_messages(Some(FolderRole::Inbox), limit)?);
            Ok(())
        }
        Command::Search { query, limit } => {
            let store = Store::open(&db_path)?;
            print_messages(&store.search(&query.join(" "), limit)?);
            Ok(())
        }
        Command::Stats => {
            let mut store = Store::open(&db_path)?;
            print_stats(&mut store)
        }
    }
}

async fn discover(email: &str) -> Result<()> {
    let candidates = autoconfig::discover(email).await?;
    if candidates.is_empty() {
        println!("Nothing found.");
    }
    for c in candidates {
        print_candidate(&c);
    }
    Ok(())
}

fn print_candidate(c: &Candidate) {
    println!(
        "[{:?}] {}  IMAP {}:{} {} ({}, user {})",
        c.source,
        c.provider_name.as_deref().unwrap_or("-"),
        c.imap.host,
        c.imap.port,
        c.imap.security,
        c.imap.auth,
        c.imap.username
    );
    if let Some(s) = &c.smtp {
        println!(
            "{:>12}SMTP {}:{} {} ({})",
            "", s.host, s.port, s.security, s.auth
        );
    }
}

async fn add_account(
    store: &Store,
    email: &str,
    display_name: Option<String>,
    manual: Option<ServerConfig>,
    username: Option<String>,
) -> Result<()> {
    if store.account_by_email(email)?.is_some() {
        bail!("{email} is already set up");
    }

    // The flag says whether the settings come from the user or the provider;
    // guessed servers need consent before they get the password.
    let candidates: Vec<(ServerConfig, Option<ServerConfig>, bool)> = match manual {
        Some(imap) => vec![(imap, None, true)],
        None => {
            println!("Looking up server settings for {email} …");
            let found = autoconfig::discover(email).await?;
            let usable: Vec<_> = found
                .into_iter()
                .filter(|c| c.imap.auth == AuthMethod::Password)
                .map(|mut c| {
                    let trusted = c.source.is_authoritative();
                    if let Some(u) = &username {
                        c.imap.username = u.clone();
                        if let Some(s) = c.smtp.as_mut() {
                            s.username = u.clone();
                        }
                    }
                    (c.imap, c.smtp, trusted)
                })
                .collect();
            if usable.is_empty() {
                bail!(
                    "no password-based IMAP settings found. Pass --imap-host, or wait for OAuth \
                     support (needed e.g. for Microsoft 365)."
                );
            }
            usable
        }
    };

    let password = rpassword::prompt_password(format!("Password for {email}: "))?;
    let credentials = Credentials::Password(password.clone());

    let mut last_error = None;
    for (imap_cfg, smtp_cfg, trusted) in candidates {
        if !trusted && !confirm_guessed_server(&imap_cfg)? {
            println!("Skipping {}:{}.", imap_cfg.host, imap_cfg.port);
            continue;
        }
        print!(
            "Trying {}:{} ({}) … ",
            imap_cfg.host, imap_cfg.port, imap_cfg.security
        );
        match imap::probe(&imap_cfg, &credentials).await {
            Ok(caps) => {
                println!("ok");
                let features: Vec<&str> = [
                    (caps.idle, "IDLE"),
                    (caps.condstore, "CONDSTORE"),
                    (caps.qresync, "QRESYNC"),
                    (caps.special_use, "SPECIAL-USE"),
                    (caps.r#move, "MOVE"),
                    (caps.gmail, "Gmail"),
                ]
                .into_iter()
                .filter_map(|(on, name)| on.then_some(name))
                .collect();
                println!("Server features: {}", features.join(", "));

                let account = store.add_account(&Account {
                    id: 0,
                    email: email.to_string(),
                    display_name,
                    imap: imap_cfg,
                    smtp: smtp_cfg,
                })?;
                secrets::store(&account.email, SecretKind::ImapPassword, &password)
                    .context("saving the password in the system keychain")?;
                println!("Added {}. Run `halcyon sync` to fetch mail.", account.email);
                return Ok(());
            }
            Err(e) => {
                println!("failed: {e}");
                // A rejected password will not work on the next server either.
                let auth_failed = matches!(e, halcyon_core::Error::Auth(_));
                last_error = Some(e);
                if auth_failed {
                    break;
                }
            }
        }
    }
    Err(last_error
        .map(anyhow::Error::from)
        .unwrap_or_else(|| anyhow::anyhow!("no server worked")))
}

/// Asks before sending the password to a server that was guessed rather than
/// published by the provider.
fn confirm_guessed_server(server: &ServerConfig) -> Result<bool> {
    use std::io::Write;
    print!(
        "{}:{} ({}) is a guess, not published by your provider. Send your password there? [y/N] ",
        server.host, server.port, server.security
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_lowercase().as_str(),
        "y" | "yes" | "j" | "ja"
    ))
}

async fn run_sync(store: &mut Store, only: Option<&str>) -> Result<()> {
    let accounts: Vec<Account> = store
        .accounts()?
        .into_iter()
        .filter(|a| only.is_none_or(|e| a.email.eq_ignore_ascii_case(e)))
        .collect();
    if accounts.is_empty() {
        bail!("no matching accounts – add one with `halcyon account add <email>`");
    }

    for account in accounts {
        println!("== {}", account.email);
        let Some(password) = secrets::load(&account.email, SecretKind::ImapPassword)? else {
            println!("   no password in the keychain, skipping");
            continue;
        };
        let report =
            sync::sync_account(store, &account, &Credentials::Password(password), |event| {
                match event {
                    SyncEvent::FolderStarted { path, new_messages } if new_messages > 0 => {
                        println!("   {path}: {new_messages} new");
                    }
                    SyncEvent::Progress { path, done, total } if total > 1000 => {
                        println!("   {path}: {done}/{total}");
                    }
                    SyncEvent::FolderReset { path } => {
                        println!("   {path}: server reset the folder, downloading again");
                    }
                    SyncEvent::FolderFailed { path, error } => {
                        println!("   {path}: FAILED – {error}");
                    }
                    _ => {}
                }
            })
            .await;
        match report {
            Ok(r) => println!(
                "   {} folders, {} new messages, {} contents downloaded{}",
                r.folders,
                r.new_messages,
                r.bodies,
                if r.failed_folders.is_empty() {
                    String::new()
                } else {
                    format!(", {} folders failed", r.failed_folders.len())
                }
            ),
            Err(e) => println!("   FAILED: {e}"),
        }
    }
    Ok(())
}

fn print_messages(messages: &[MessageSummary]) {
    if messages.is_empty() {
        println!("No messages.");
        return;
    }
    for m in messages {
        let h = &m.header;
        let date = h
            .date
            .and_then(|d| Local.timestamp_opt(d, 0).single())
            .map(|d| d.format("%d.%m.%y %H:%M").to_string())
            .unwrap_or_else(|| "–".repeat(14));
        let from = h
            .from_name
            .as_deref()
            .or(h.from_addr.as_deref())
            .unwrap_or("?");
        println!(
            "{} {date}  {:<24.24}  {:<60.60}  [{}]",
            if m.is_seen() { ' ' } else { '●' },
            from,
            h.subject.as_deref().unwrap_or("(kein Betreff)"),
            m.account_email,
        );
    }
}

fn print_stats(store: &mut Store) -> Result<()> {
    let now = Local::now().timestamp();
    let link_report = halcyon_core::linker::run(store, now)?;
    let s = store.evaluate_mailbox_stats(now)?;
    let pct = |n: usize, d: usize| -> f64 {
        if d == 0 {
            0.0
        } else {
            (n as f64 / d as f64) * 100.0
        }
    };

    println!("=== Halcyon – Lokale Auswertung (nur Zahlen, keine Inhalte) ===");
    println!(
        "Verknüpfer-Lauf: {} geprüft, {} verknüpft, {} neu, {} abgeschlossen",
        link_report.scanned, link_report.linked, link_report.created, link_report.closed
    );
    println!(
        "Nachrichten gesamt (ohne Spam/Papierkorb/Entwürfe): {}",
        s.total_messages
    );
    println!(
        "Quellen: {} · Betreffmuster: {} · Wiederholungen (>=2): {} ({:.1}%)",
        s.unique_sources,
        s.unique_patterns,
        s.recurring_pattern_messages,
        pct(s.recurring_pattern_messages, s.total_messages)
    );
    println!(
        "Abdeckung Top-100-Muster: {} Mails ({:.1}%)",
        s.top_100_patterns_messages,
        pct(s.top_100_patterns_messages, s.total_messages)
    );
    println!(
        "Quellen-Modi: anzeigen={} · zusammenfassen={} · nur Stand={} · stumm={} · Abweichungen={}",
        s.sources_show, s.sources_bundle, s.sources_latest_only, s.sources_mute, s.active_anomalies
    );
    println!(
        "Vorgänge: braucht dich={} · wartet={} · erledigt={} (Bestellungen={}, Rechnungen={}, Verträge={})",
        s.cases_needs_you,
        s.cases_waiting,
        s.cases_done,
        s.cases_order,
        s.cases_invoice,
        s.cases_contract
    );
    println!(
        "Gespräche (Threads): gesamt={} · braucht dich={} · wartet={} (Nachhaken={}) · mitlesen={} · erledigt={}",
        s.human_threads_total,
        s.human_needs_you,
        s.human_waiting,
        s.human_follow_up_due,
        s.human_reading_along,
        s.human_done
    );
    println!(
        "Verdichtung (letzte 7 Tage): {} Zeilen auf 'Heute' bei {} eingehenden Mails ({:.1}%)",
        s.recent_7d_today_rows,
        s.recent_7d_incoming,
        pct(s.recent_7d_today_rows, s.recent_7d_incoming)
    );
    Ok(())
}
