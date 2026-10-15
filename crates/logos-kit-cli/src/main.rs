//! `logos-kit`: the Logos Kit wallet on the command line.
//!
//! Every command runs through the same engine and policy as the Basecamp
//! wallet, as the `LocalOwner` caller: a transaction is built, shown decoded
//! with its fee cap, confirmed on the terminal, and only then approved.
//!
//! Non-interactive use (demo scripts, E2E): set `LOGOS_KIT_PASSWORD`; only
//! then does `--yes` skip the confirmation.

use std::{
    io::{BufRead as _, IsTerminal as _, Write as _},
    path::PathBuf,
    time::Instant,
};

use anyhow::{Context as _, Result, bail, ensure};
use clap::{Parser, Subcommand};
use wallet_engine::AccountId;
use wallet_engine::{
    engine::{Config, Engine, Lifecycle, RequestView, Ticket, TxStatus},
    faucet::{FundOutcome, HttpFaucet, KeyFaucet},
    policy::{Caller, code_of},
    session::{AccountKind, Birthday, DataDir, Session, Zone},
    testimonial,
    tx::{CallAccount, Intent, RecipientKeys, Review, Route},
    vault::KdfCost,
    verify,
};

#[derive(Parser)]
#[command(name = "logos-kit", version, about = "Logos Kit wallet for LEZ")]
struct Cli {
    /// Wallet data directory.
    #[arg(id = "home", long = "home", global = true, env = "LOGOS_KIT_HOME")]
    data: Option<PathBuf>,
    /// Zone id: `lez-testnet` (the official LEZ testnet), `lez-preview` (Logos Kit's own 0.3 network), `lez-local`, or one added with --sequencer.
    #[arg(
        long,
        global = true,
        env = "LOGOS_KIT_ZONE",
        default_value = "lez-testnet"
    )]
    zone: String,
    /// Sequencer URL, to add a zone that isn't built in.
    #[arg(long, global = true)]
    sequencer: Option<String>,
    /// Skip the confirmation (only with LOGOS_KIT_PASSWORD set).
    #[arg(long, global = true)]
    yes: bool,
    /// Print results as JSON.
    #[arg(long, global = true)]
    json: bool,
    /// With --yes: also approve a call the wallet can't decode.
    #[arg(long, global = true)]
    ack_unknown: bool,
    /// Prove private transactions in smaller pieces: about half the memory,
    /// and slower. `LOGOS_KIT_LOW_MEMORY=1` (or true, yes, on) does the same.
    #[arg(long, global = true, env = "LOGOS_KIT_LOW_MEMORY", value_parser = clap::builder::FalseyValueParser::new())]
    low_memory: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new wallet and show its recovery phrase once.
    Init,
    /// Restore a wallet from a recovery phrase (asked on the terminal).
    Restore {
        /// First used around this date (YYYY-MM-DD); earlier blocks are skipped.
        #[arg(long, conflicts_with = "from_genesis")]
        from_date: Option<String>,
        /// Scan the whole chain (slow, always complete).
        #[arg(long)]
        from_genesis: bool,
    },
    /// Accounts.
    #[command(subcommand)]
    Account(AccountCmd),
    /// Balance of an account (public, or private as synced); `--token` for a
    /// token. The native token prints in LGO (`--json`: lepta).
    Balance {
        account: String,
        #[arg(long)]
        token: Option<String>,
    },
    /// Catch up with the chain.
    Sync,
    /// Send native tokens or a token. The route follows from the accounts:
    /// public → public, into your private account (shield), out of it
    /// (unshield), or private → private (proves locally, minutes).
    Send(SendArgs),
    /// Send from a public account into one of your private accounts.
    Shield(SendArgs),
    /// Send from one of your private accounts to a public account.
    #[command(alias = "unshield")]
    Deshield(SendArgs),
    /// Tokens.
    #[command(subcommand)]
    Token(TokenCmd),
    /// Get testnet funds into one of your accounts (a private one is funded
    /// through a public account, then shielded).
    Faucet {
        account: String,
        /// Public account to fund first when the target is private.
        #[arg(long)]
        via: Option<String>,
        /// Drip service URL (the zone's faucet).
        #[arg(long, env = "LOGOS_KIT_FAUCET_URL", conflicts_with = "key_env")]
        url: Option<String>,
        /// Name of an env var holding a funded key to pay from (local/demo).
        #[arg(long)]
        key_env: Option<String>,
        /// Lepta per claim when paying from a key (1 LGO = 1000000000 lepta).
        #[arg(long, default_value_t = 1_000_000)]
        drop: u128,
    },
    /// Show a program's header and verification status (address, or a builtin's name).
    /// With --name, also give it your own name: approvals show "<name> (named by you)".
    Program {
        account: String,
        /// Your name for this program on this network (shown on approvals).
        #[arg(long)]
        name: Option<String>,
        /// Forget the name you gave it.
        #[arg(long, conflicts_with = "name")]
        forget: bool,
    },
    /// Programs you named on this network.
    Programs,
    /// Rebuild a program from source (docker) and compare with what's deployed.
    VerifyProgram {
        /// Program account; its source comes from the registry unless given.
        account: Option<String>,
        /// Rebuild the LEZ builtins we decode (token, ATA) from vendor/lez or --repo.
        #[arg(long, conflicts_with = "account")]
        builtins: bool,
        #[arg(long)]
        repo: Option<String>,
        #[arg(long)]
        commit: Option<String>,
        #[arg(long)]
        guest_path: Option<String>,
        #[arg(long)]
        bin: Option<String>,
        #[arg(long)]
        features: Option<String>,
        #[arg(long, default_value = "r0.1.91.1")]
        docker_tag: String,
    },
    /// Call any program (what a dApp proposes). Accounts: `id[:shard][:signer]`.
    Call {
        #[arg(long)]
        from: String,
        #[arg(long)]
        program: String,
        #[arg(long = "account", required = true)]
        accounts: Vec<String>,
        /// Instruction bytes, base64.
        #[arg(long)]
        data: String,
    },
    /// Encrypted backups (needs the password they were made with to restore).
    #[command(subcommand)]
    Backup(BackupCmd),
    /// The testimonial program (LP-0021): post, list, evidence, build, deploy.
    #[command(subcommand)]
    Testimonial(TestimonialCmd),
    /// Zone, sync and network state.
    Status,
    /// Zones this wallet knows.
    Zones,
    /// Show the recovery phrase (asks for the password again).
    Reveal,
    /// Change the wallet password.
    Password,
    /// Lock after this many seconds without use (60–86400).
    AutoLock { seconds: u32 },
}

#[derive(clap::Args, Clone)]
struct SendArgs {
    #[arg(long)]
    from: String,
    /// Recipient account (yours or public), or a `.lez` name (LNS), which pays
    /// the private keys or public account the name publishes. For someone
    /// else's private account without a name use --to-keys (or --to-npk/--to-vpk).
    #[arg(long)]
    to: Option<String>,
    /// Native: lepta as a whole number (1 LGO = 1000000000 lepta), or LGO
    /// with a decimal point or an `LGO` suffix (`2.5`, `2.5LGO`, `3LGO`).
    /// With --token: a whole number of the token's base units.
    #[arg(long)]
    amount: String,
    /// Token definition account; omit for the native token.
    #[arg(long)]
    token: Option<String>,
    #[arg(long, requires = "to_vpk")]
    to_npk: Option<String>,
    #[arg(long, requires = "to_npk")]
    to_vpk: Option<String>,
    /// Keys file from `logos-kit account keys` (npk, vpk lines).
    #[arg(long, conflicts_with_all = ["to_npk", "to_vpk"])]
    to_keys: Option<PathBuf>,
    #[arg(long)]
    to_identifier: Option<String>,
}

#[derive(Subcommand)]
enum TokenCmd {
    /// Tokens in your accounts (own slots and token accounts). Reads new
    /// blocks first, so tokens someone sent you show up. By default: the
    /// Logos Kit list's tokens and the ones you added.
    List {
        /// Every tier, hidden and spam included.
        #[arg(long)]
        all: bool,
        /// Tokens that arrived but aren't on the list (the app's "Unknown").
        #[arg(long)]
        unknown: bool,
        /// Tokens hidden as spam (with the reason).
        #[arg(long)]
        spam: bool,
        /// Tokens you hid.
        #[arg(long)]
        hidden: bool,
    },
    /// Add a token by its ID: shows what it is first; asks before adding.
    Add {
        definition: String,
        /// Display decimals, for a token the list doesn't describe (0–36).
        #[arg(long)]
        decimals: Option<u8>,
        /// Add it although it looks like a verified token or LGO.
        #[arg(long)]
        accept_lookalike: bool,
    },
    /// Stop listing a token you added (it doesn't touch your balance).
    Remove {
        definition: String,
    },
    /// Hide a token (nothing is deleted; `unhide` brings it back).
    Hide {
        definition: String,
    },
    Unhide {
        definition: String,
    },
    /// Keep a token near the top of the list.
    Pin {
        definition: String,
    },
    Unpin {
        definition: String,
    },
    /// What a token is: name, supply, decimals and their source, trust, your balance.
    Info {
        definition: String,
    },
    /// Same as `add` without the checks (kept for scripts).
    #[command(hide = true)]
    Track {
        definition: String,
    },
    /// The associated token account address of `owner` for a token.
    Ata {
        owner: String,
        definition: String,
    },
    /// Create a fungible token; all supply goes to --holder (a public account
    /// whose token slot is empty). A new account becomes the definition.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long)]
        supply: u128,
        #[arg(long)]
        holder: String,
    },
}

#[derive(Subcommand)]
enum TestimonialCmd {
    /// Post a testimonial from one of your public accounts (it pays the fee
    /// and is shown on-chain as the author).
    Post {
        #[arg(long)]
        from: String,
        #[arg(long)]
        text: String,
        #[arg(long)]
        username: Option<String>,
        /// Program account (default: the registry's for this zone).
        #[arg(long)]
        program: Option<String>,
        #[arg(long, default_value = testimonial::SUBMISSION)]
        submission: String,
    },
    /// A submission's testimonials, in posting order.
    List {
        #[arg(long)]
        program: Option<String>,
        #[arg(long, default_value = testimonial::SUBMISSION)]
        submission: String,
    },
    /// Adoption evidence from chain data: distinct authors per month, the
    /// LP-0021 target, and each author's own transactions before its post
    /// (prior activity, read from the blocks). Repeat --program for a
    /// redeploy or a chain reset.
    Evidence {
        #[arg(long = "program")]
        programs: Vec<String>,
        #[arg(long, default_value = testimonial::SUBMISSION)]
        submission: String,
        /// Block-scan cache to resume from (default: `<snapshot>/.blocks-<zone>.json`
        /// with --snapshot, none otherwise).
        #[arg(long)]
        cache: Option<PathBuf>,
        /// Also write `<dir>/<submission>-<date>.json` (records + tip block).
        #[arg(long)]
        snapshot: Option<PathBuf>,
    },
    /// Build the program from a commit of this repo in the pinned docker
    /// builder; write `testimonial.bin` and `build.json` to --out.
    Build {
        /// Default: HEAD.
        #[arg(long)]
        commit: Option<String>,
        /// The public repo recorded as the source.
        #[arg(long, default_value = "https://github.com/logos-kit/logos-kit")]
        repo_url: String,
        #[arg(long, default_value = "r0.1.91.1")]
        docker_tag: String,
        #[arg(long, default_value = "programs/testimonial/artifacts")]
        out: PathBuf,
    },
    /// Deploy the built program (owner). New accounts of this wallet hold the
    /// header and segments; --payer pays.
    Deploy {
        #[arg(long)]
        payer: String,
        /// Keep an upgrade key (staging only: the wallet trusts an
        /// upgradeable copy only if the registry names it).
        #[arg(long)]
        upgradeable: bool,
        #[arg(long, default_value = "programs/testimonial/artifacts/testimonial.bin")]
        bin: PathBuf,
    },
    /// Print the `call` arguments of a post to a chosen stats page, one per
    /// line (`--account …`, `--data …`). Tests the wallet's page retry.
    #[command(hide = true)]
    CallArgs {
        #[arg(long)]
        program: String,
        #[arg(long)]
        from: String,
        #[arg(long)]
        page: u32,
        #[arg(long)]
        text: String,
        #[arg(long, default_value = testimonial::SUBMISSION)]
        submission: String,
    },
}

#[derive(Subcommand)]
enum BackupCmd {
    /// Write an encrypted backup of this wallet.
    Export { file: PathBuf },
    /// Restore a backup into an empty data dir (checks the password).
    Import { file: PathBuf },
}

#[derive(Subcommand)]
enum AccountCmd {
    List,
    New {
        #[arg(long)]
        private: bool,
    },
    /// Name an account; without a name, clear it.
    Label {
        account: String,
        name: Option<String>,
    },
    /// Import a public account by private key (asked on the terminal).
    Import,
    /// Keys others need to pay a private account privately (npk, vpk lines;
    /// save to a file and share it; it holds no secret).
    Keys {
        account: String,
    },
}

fn data_dir(cli: &Cli) -> Result<DataDir> {
    if let Some(dir) = &cli.data {
        return Ok(DataDir::new(dir));
    }
    let home = std::env::var_os("HOME").context("HOME is not set; pass --home")?;
    Ok(DataDir::new(PathBuf::from(home).join(".logos-kit")))
}

fn zone(cli: &Cli, data: &DataDir) -> Result<Zone> {
    if let Some(url) = &cli.sequencer {
        return Ok(Zone {
            id: cli.zone.clone(),
            chain: format!("lez:{}", cli.zone.trim_start_matches("lez-")),
            sequencer: url.clone(),
        });
    }
    for z in Zone::builtin() {
        if z.id == cli.zone {
            return Ok(z);
        }
    }
    data.zones()?
        .into_iter()
        .find(|z| z.id == cli.zone)
        .with_context(|| format!("unknown zone {}; add it with --sequencer <url>", cli.zone))
}

fn noninteractive() -> Option<String> {
    std::env::var("LOGOS_KIT_PASSWORD").ok()
}

fn password(prompt: &str) -> Result<String> {
    match noninteractive() {
        Some(pw) => Ok(pw),
        None => Ok(rpassword::prompt_password(prompt)?),
    }
}

fn new_password() -> Result<String> {
    if let Some(pw) = noninteractive() {
        return Ok(pw);
    }
    let pw = rpassword::prompt_password("New password: ")?;
    ensure!(pw.chars().count() >= 8, "use at least 8 characters");
    ensure!(
        rpassword::prompt_password("Repeat it: ")? == pw,
        "the passwords don't match"
    );
    Ok(pw)
}

fn confirm(cli: &Cli, question: &str) -> Result<bool> {
    if cli.yes {
        ensure!(
            noninteractive().is_some(),
            "--yes only works non-interactively (LOGOS_KIT_PASSWORD set)"
        );
        return Ok(true);
    }
    ensure!(
        std::io::stdin().is_terminal(),
        "no terminal to confirm on; use --yes with LOGOS_KIT_PASSWORD for scripts"
    );
    print!("{question} [y/N] ");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}

/// YYYY-MM-DD (UTC midnight) → unix ms (days-from-civil, H. Hinnant).
fn date_ms(date: &str) -> Result<u64> {
    let parts: Vec<i64> = date
        .split('-')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .context("date must be YYYY-MM-DD")?;
    let [y, m, d] = parts[..] else {
        bail!("date must be YYYY-MM-DD");
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days_in = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    ensure!(
        (1..=12).contains(&m) && d >= 1 && d <= days_in[usize::try_from(m - 1)?],
        "no such date: {date}"
    );
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400_000).context("date before 1970")
}

fn print(cli: &Cli, value: &serde_json::Value, human: impl FnOnce()) {
    if cli.json {
        println!("{value}");
    } else {
        human();
    }
}

/// The native token is lepta on the wire and LGO on screen (the official
/// Logos convention): 1 LGO = 10^9 lepta. JSON output stays in lepta.
const LEPTA_PER_LGO: u128 = 1_000_000_000;

/// Lepta → LGO, split in integers: nothing rounded, trailing zeros cut
/// (1 → "0.000000001", 1500000000 → "1.5").
fn lgo(lepta: u128) -> String {
    let (whole, frac) = (lepta / LEPTA_PER_LGO, lepta % LEPTA_PER_LGO);
    if frac == 0 {
        return whole.to_string();
    }
    let frac = format!("{frac:09}");
    format!("{whole}.{}", frac.trim_end_matches('0'))
}

/// `--amount` → base units. A whole number is lepta (or a token's base
/// units), as before; a decimal point or an `LGO` suffix means LGO, at most
/// 9 decimals (refused past that, never rounded).
fn parse_amount(text: &str, token: bool) -> Result<u128> {
    let t = text.trim();
    let (num, suffixed) = match t.len().checked_sub(3) {
        Some(i) if t.is_char_boundary(i) && t[i..].eq_ignore_ascii_case("lgo") => {
            (t[..i].trim_end(), true)
        }
        _ => (t, false),
    };
    if !suffixed && !num.contains('.') {
        return num.parse().with_context(|| {
            format!("--amount {text}: not a whole number of lepta (for LGO write 2.5 or 2.5LGO)")
        });
    }
    ensure!(
        !token,
        "--amount {text}: a token's amount is a whole number of its base units (LGO is the native token)"
    );
    let (whole, frac) = num.split_once('.').unwrap_or((num, ""));
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    ensure!(
        !(whole.is_empty() && frac.is_empty()) && digits(whole) && digits(frac),
        "--amount {text}: not an amount of LGO"
    );
    ensure!(
        frac.len() <= 9,
        "--amount {text}: LGO has 9 decimals (0.000000001 LGO is 1 lepta)"
    );
    let whole: u128 = if whole.is_empty() { 0 } else { whole.parse()? };
    let frac: u128 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<9}").parse()?
    };
    whole
        .checked_mul(LEPTA_PER_LGO)
        .and_then(|w| w.checked_add(frac))
        .with_context(|| format!("--amount {text}: too large"))
}

async fn open(cli: &Cli) -> Result<(Session, String)> {
    let data = data_dir(cli)?;
    ensure!(
        data.is_initialized(),
        "no wallet in {}; run `logos-kit init` or `logos-kit restore`",
        data.root().display()
    );
    let zone = zone(cli, &data)?;
    let pw = password("Password: ")?;
    Ok((Session::unlock(data, &pw, zone)?, pw))
}

fn describe(status: &TxStatus, started: Instant) -> String {
    let what = match status.lifecycle {
        Lifecycle::Building => "building",
        Lifecycle::Proving => "proving locally (about 5–6 min on a desktop CPU; keep this open)",
        Lifecycle::Signing => "signing",
        Lifecycle::Submitted => "submitted, waiting for a block",
        Lifecycle::Included => "included",
        Lifecycle::Dropped => "not sent",
        Lifecycle::Rejected => "declined",
        Lifecycle::Expired => "expired",
        Lifecycle::AwaitingApproval => "waiting for approval",
    };
    format!("[{:>4}s] {what}", started.elapsed().as_secs())
}

fn show_review(review: &Review) {
    let sum = &review.summary;
    let route = match review.route {
        Some(Route::Public) | None => "",
        Some(Route::Shield) => "  (public → your private account; proves locally)",
        Some(Route::Unshield) => "  (private → public; proves locally)",
        Some(Route::Private) => "  (private; proves locally)",
    };
    println!("{} on {}{route}", sum.title, review.chain);
    if let Some(app) = &review.requester {
        println!("  asked by {app}");
    }
    // `from_balance` is the token's for a token transfer, else lepta.
    let balance = match &review.intent {
        Intent::Transfer { token: Some(_), .. } => review.from_balance.to_string(),
        _ => format!("{} LGO", lgo(review.from_balance)),
    };
    println!(
        "  from      {}  (balance {balance})",
        review.intent.from_account(),
    );
    if let Some(to) = &review.recipient {
        println!("  to        {to}");
    }
    for line in &sum.lines {
        // The engine writes a native transfer as "<lepta> to <account>".
        match line.split_once(" to ") {
            Some((n, to)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => {
                match n.parse::<u128>() {
                    Ok(v) => println!("  · {} LGO to {to}", lgo(v)),
                    Err(_) => println!("  · {line}"),
                }
            }
            _ => println!("  · {line}"),
        }
    }
    for a in &sum.authorities {
        println!("  ⚠ AUTHORITY  {a}");
    }
    if sum.unknown {
        println!("  ⚠ UNKNOWN  the wallet can't read what this call does");
    }
    if let Some(p) = &review.program {
        println!(
            "  program   {} [{:?}] {}",
            p.name.as_deref().unwrap_or(&p.account),
            p.status,
            p.note
        );
    }
    for e in &review.expected_effects {
        println!(
            "  public    {}",
            serde_json::to_string(e).unwrap_or_default()
        );
    }
    let as_lgo = |v: &String| {
        v.parse::<u128>()
            .map_or_else(|_| format!("{v} lepta"), |l| format!("{} LGO", lgo(l)))
    };
    match (&review.fee.max_fee, &review.fee.payer) {
        (Some(max), Some(payer)) => {
            match &review.fee.estimate {
                Some(now) => println!(
                    "  fee       {}{} at today's rate (cap {}), paid by {payer}",
                    if review.fee.exact { "" } else { "up to " },
                    as_lgo(now),
                    as_lgo(max)
                ),
                None => println!("  fee       up to {}, paid by {payer}", as_lgo(max)),
            }
            if let Some(base) = review.fee.base_fee_exec {
                println!("            network base fee now {base} lepta per gas");
            }
        }
        _ => println!("  fee       none (private transactions are fee-exempt)"),
    }
    println!("  request   {}", review.request_hash);
}

/// Review, confirm, approve and follow one request. `expect` refuses a
/// request whose route isn't the one the command promised.
async fn transact(cli: &Cli, intent: Intent, expect: Option<Route>) -> Result<()> {
    // JSON output leaves no room for the review; scripts confirm with --yes.
    ensure!(
        !cli.json || cli.yes,
        "--json needs --yes (run without --json to review first)"
    );
    let (session, pw) = open(cli).await?;
    let engine = Engine::new(session, Config::default());
    let owner = Caller::LocalOwner;
    let ticket = engine.request_tx(&owner, None, intent).await?;
    let approved = approve_ticket(cli, &engine, &pw, ticket, expect).await;
    // Lock (which writes the activity history) whether or not it went through.
    engine.lock().await?;
    approved.map(|_| ())
}

async fn approve_ticket(
    cli: &Cli,
    engine: &Engine,
    pw: &str,
    ticket: Ticket,
    expect: Option<Route>,
) -> Result<TxStatus> {
    let owner = Caller::LocalOwner;
    let RequestView::Transaction(review) = &ticket.request else {
        bail!("unexpected request type");
    };
    if let Some(want) = expect
        && review.route != Some(want)
    {
        engine.reject(&owner, &ticket.handle)?;
        bail!(
            "these accounts make this a {:?} transfer, not {want:?}; use `logos-kit send`",
            review.route
        );
    }
    if !cli.json {
        show_review(review);
    }
    if !confirm(cli, "Approve?")? {
        engine.reject(&owner, &ticket.handle)?;
        bail!("declined");
    }
    let acknowledged = if ticket.needs_acknowledgement {
        if cli.yes {
            ensure!(
                cli.ack_unknown,
                "this calls a program the wallet can't read; add --ack-unknown to approve it"
            );
            true
        } else {
            confirm(cli, "The wallet can't tell what this does. Approve anyway?")?
        }
    } else {
        false
    };
    let started = Instant::now();
    let json = cli.json;
    let mut progress = move |s: &TxStatus| {
        if !json {
            println!("{}", describe(s, started));
        }
    };
    let approved = engine
        .approve(
            &owner,
            &ticket.handle,
            &review.request_hash,
            Some(pw),
            acknowledged,
            &mut progress,
        )
        .await;
    let status = match approved {
        Ok(status) => status,
        Err(e) => {
            // Say how far it got: a submitted tx may still land; don't resend blindly.
            if let Ok(s) = engine.status(&owner, &ticket.handle)
                && let Some(hash) = &s.tx_hash
            {
                eprintln!("tx {hash} was submitted; check it before sending again");
            }
            if !acknowledged && ticket.needs_acknowledgement {
                let _ = engine.reject(&owner, &ticket.handle);
            }
            return Err(e);
        }
    };
    if let Some(warning) = &status.error {
        eprintln!("note: {warning}");
    }
    print(cli, &serde_json::to_value(&status)?, || {
        println!(
            "tx {} in block {} (outcome: {:?})",
            status.tx_hash.as_deref().unwrap_or("?"),
            status.block.map_or("?".into(), |b| b.to_string()),
            status.outcome
        );
        if let Some(paid) = status
            .fee_paid
            .as_deref()
            .and_then(|f| f.parse::<u128>().ok())
        {
            println!("fee paid {} LGO", lgo(paid));
        }
    });
    Ok(status)
}

fn transfer(args: &SendArgs) -> Result<Intent> {
    let to_keys = match (&args.to_keys, &args.to_npk, &args.to_vpk) {
        (Some(path), _, _) => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("keys file {}", path.display()))?;
            let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
            Some(RecipientKeys {
                npk: lines
                    .next()
                    .context("keys file: npk line missing")?
                    .to_owned(),
                vpk: lines
                    .next()
                    .context("keys file: vpk line missing")?
                    .to_owned(),
                identifier: args.to_identifier.clone(),
            })
        }
        (None, Some(npk), Some(vpk)) => Some(RecipientKeys {
            npk: npk.clone(),
            vpk: vpk.clone(),
            identifier: args.to_identifier.clone(),
        }),
        _ => None,
    };
    Ok(Intent::Transfer {
        from: args.from.clone(),
        to: args.to.clone(),
        amount: parse_amount(&args.amount, args.token.is_some())?,
        token: args.token.clone(),
        to_keys,
    })
}

fn call_account(spec: &str) -> Result<CallAccount> {
    let mut parts = spec.split(':');
    let account = parts.next().context("empty account")?.to_owned();
    let mut shard = None;
    let mut signer = false;
    for p in parts {
        if p == "signer" {
            signer = true;
        } else {
            ensure!(shard.is_none(), "{spec}: more than one shard");
            shard = Some(p.to_owned());
        }
    }
    Ok(CallAccount {
        account,
        shard,
        signer,
    })
}

fn write_private(path: &std::path::Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .with_context(|| format!("{} (it must not exist yet)", path.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

/// A token preview, as `token add` and `token info` show it.
fn print_preview(cli: &Cli, p: &wallet_engine::tokens::TokenPreview) {
    print(cli, &serde_json::to_value(p).unwrap_or_default(), || {
        let supply = match p.info.decimals {
            Some(d) => wallet_engine::tokens::format_units(p.total_supply, d),
            None => format!("{} (decimals unknown)", p.total_supply),
        };
        println!("{}  {}", p.name, p.definition);
        println!(
            "  trust     {:?}{}",
            p.info.tier,
            p.listed_in
                .as_deref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default()
        );
        if let Some(r) = &p.info.spam_reason {
            println!("  warning   {r}");
        }
        if let Some(i) = &p.imitates {
            println!("  warning   looks like {} but isn't", i.name);
        }
        println!("  supply    {supply}");
        println!(
            "  decimals  {} ({})",
            p.info
                .decimals
                .map(|d| d.to_string())
                .unwrap_or_else(|| "unknown".into()),
            p.info.decimals_source
        );
        println!(
            "  yours     {}",
            match p.info.decimals {
                Some(d) => wallet_engine::tokens::format_units(p.your_balance, d),
                None => p.your_balance.to_string(),
            }
        );
        if let Some(m) = &p.metadata {
            println!(
                "  metadata  {} {} (creators: {})",
                m.standard, m.uri, m.creators
            );
        }
    });
}

async fn token(cli: &Cli, cmd: &TokenCmd) -> Result<()> {
    match cmd {
        TokenCmd::List {
            all,
            unknown,
            spam,
            hidden,
        } => {
            use wallet_engine::trust::Tier;
            let (session, _) = open(cli).await?;
            let engine = Engine::new(session, Config::default());
            // Read the blocks since the last look, so tokens someone sent
            // show up (a restored wallet may need several steps).
            for _ in 0..200 {
                engine.with_session(async |s| s.connect().await).await?;
                if engine.scan_incoming().await.is_err() || engine.scan_behind() == 0 {
                    break;
                }
            }
            // Holdings read the scan's findings from the history record.
            engine.flush_history().await?;
            let holdings = engine.with_session(async |s| s.holdings().await).await?;
            engine.lock().await?;
            let wanted = |t: Tier| {
                *all || match t {
                    Tier::Verified | Tier::Added => !(*unknown || *spam || *hidden),
                    Tier::Unknown => *unknown,
                    Tier::Spam => *spam,
                    Tier::Hidden => *hidden,
                }
            };
            let shown: Vec<_> = holdings
                .into_iter()
                .filter(|h| wanted(h.info.tier))
                .collect();
            print(cli, &serde_json::to_value(&shown)?, || {
                if shown.is_empty() {
                    println!("no tokens");
                }
                for h in &shown {
                    let amount = match h.info.decimals {
                        Some(d) => wallet_engine::tokens::format_units(h.amount, d),
                        None => format!("{} (decimals unknown)", h.amount),
                    };
                    println!(
                        "{:<46} {:>24} {:<16} {:<9} {} {}{}",
                        h.account,
                        amount,
                        h.info
                            .symbol
                            .as_deref()
                            .or(h.name.as_deref())
                            .unwrap_or("?"),
                        format!("{:?}", h.info.tier).to_lowercase(),
                        h.definition,
                        if h.private { "(private)" } else { "" },
                        h.info
                            .spam_reason
                            .as_deref()
                            .map(|r| format!(" [{r}]"))
                            .unwrap_or_default(),
                    );
                }
            });
            Ok(())
        }
        TokenCmd::Add {
            definition,
            decimals,
            accept_lookalike,
        } => {
            let (mut session, _) = open(cli).await?;
            let network = session.zone().id.clone();
            let preview = match session.lookup_token(definition).await? {
                Ok(p) => p,
                Err(problem) => {
                    session.lock()?;
                    bail!("{}", problem.text(&network));
                }
            };
            print_preview(cli, &preview);
            if let Some(i) = &preview.imitates {
                ensure!(
                    *accept_lookalike,
                    "this looks like {} but isn't{}; add --accept-lookalike to add it anyway",
                    i.name,
                    i.definition
                        .as_deref()
                        .map(|d| format!(" (the real one is {d})"))
                        .unwrap_or_default()
                );
            }
            if !cli.json {
                println!("Anyone can create a token, including fake copies of real ones.");
            }
            if !confirm(cli, "Add this token?")? {
                session.lock()?;
                bail!("not added");
            }
            session.add_token(definition, *decimals)?;
            if !cli.json {
                println!("added {}", preview.name);
            }
            session.lock()
        }
        TokenCmd::Remove { definition } => {
            let (mut session, _) = open(cli).await?;
            session.untrack_token(definition)?;
            session.lock()
        }
        TokenCmd::Hide { definition } | TokenCmd::Unhide { definition } => {
            let (mut session, _) = open(cli).await?;
            session.set_token_hidden(definition, matches!(cmd, TokenCmd::Hide { .. }))?;
            session.lock()
        }
        TokenCmd::Pin { definition } | TokenCmd::Unpin { definition } => {
            let (mut session, _) = open(cli).await?;
            session.set_token_pinned(definition, matches!(cmd, TokenCmd::Pin { .. }))?;
            session.lock()
        }
        TokenCmd::Info { definition } => {
            let (mut session, _) = open(cli).await?;
            let network = session.zone().id.clone();
            let result = session.lookup_token(definition).await?;
            session.lock()?;
            match result {
                Ok(p) => {
                    print_preview(cli, &p);
                    Ok(())
                }
                Err(problem) => bail!("{}", problem.text(&network)),
            }
        }
        TokenCmd::Ata { owner, definition } => {
            let ata = wallet_engine::tokens::ata_of(
                wallet_engine::decode::account_id(owner)?,
                wallet_engine::decode::account_id(definition)?,
            );
            print(cli, &serde_json::json!({ "ata": ata.to_string() }), || {
                println!("{ata}");
            });
            Ok(())
        }
        TokenCmd::Track { definition } => {
            let (mut session, _) = open(cli).await?;
            wallet_engine::decode::account_id(definition)?;
            session.track_token(definition)?;
            session.lock()
        }
        TokenCmd::Create {
            name,
            supply,
            holder,
        } => {
            ensure!(
                !cli.json || cli.yes,
                "--json needs --yes (run without --json to review first)"
            );
            let (mut session, pw) = open(cli).await?;
            let definition = session
                .new_system_account(&format!("{name} token ID"))?
                .account_id;
            let intent =
                wallet_engine::tokens::create_token_intent(holder, &definition, name, *supply)?;
            let engine = Engine::new(session, Config::default());
            let owner = Caller::LocalOwner;
            let ticket = engine.request_tx(&owner, None, intent).await?;
            let approved = approve_ticket(cli, &engine, &pw, ticket, None).await;
            if approved.is_err() {
                engine.lock().await?;
            }
            let status = approved?;
            engine
                .with_session(async |s| s.track_token(&definition))
                .await?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "definition": definition, "status": status })
                );
            } else {
                println!("token definition {definition}");
            }
            engine.lock().await
        }
    }
}

async fn faucet(
    cli: &Cli,
    account: &str,
    via: Option<&str>,
    url: Option<&str>,
    key_env: Option<&str>,
    drop: u128,
) -> Result<()> {
    let (session, pw) = open(cli).await?;
    let sequencer = session.zone().sequencer.clone();
    // The testnet and the preview network ship with a drip faucet (unless a key is given).
    let url = url.or_else(|| {
        key_env
            .is_none()
            .then(|| session.zone().builtin_faucet())
            .flatten()
    });
    let engine = Engine::new(session, Config::default());
    let owner = Caller::LocalOwner;
    let mut key = [0u8; 16];
    getrandom::fill(&mut key).map_err(|e| anyhow::anyhow!("{e}"))?;
    let request_key = hex::encode(key);
    let funds = match (url, key_env) {
        (Some(url), _) => {
            let f = HttpFaucet::new("Logos Kit drip", url)?;
            engine
                .request_funds(&owner, None, account, via, &f, &request_key)
                .await?
        }
        (None, Some(var)) => {
            let secret = std::env::var(var).with_context(|| format!("{var} is not set"))?;
            let f = KeyFaucet::new(
                "Local funded key",
                &sequencer,
                &secret,
                drop,
                std::time::Duration::from_secs(60),
            )?;
            engine
                .request_funds(&owner, None, account, via, &f, &request_key)
                .await?
        }
        (None, None) => bail!(
            "this zone has no faucet configured: pass --url <drip service> (or --key-env for a local funded key)"
        ),
    };
    let shield = funds.shield.clone();
    print(cli, &serde_json::to_value(&funds)?, || {
        match &funds.outcome {
            FundOutcome::Funded { amount, tx_hash } => println!(
                "{}: funded {} LGO into {} (tx {tx_hash})",
                funds.faucet,
                lgo(*amount),
                funds.funded_account
            ),
            outcome => println!(
                "{}: {}",
                funds.faucet,
                serde_json::to_string(outcome).unwrap_or_default()
            ),
        }
    });
    if let Some(ticket) = shield {
        if !cli.json {
            println!(
                "funded {}; now shielding into {account}",
                funds.funded_account
            );
        }
        approve_ticket(cli, &engine, &pw, ticket, Some(Route::Shield)).await?;
    }
    engine.lock().await
}

/// Rebuild the LEZ builtins we decode and print the evidence JSON
/// (`registry/builtins.json`).
fn verify_builtins(repo: Option<&str>, docker_tag: &str) -> Result<()> {
    let source = verify::Source {
        repo: repo
            .unwrap_or("https://github.com/logos-blockchain/logos-execution-zone")
            .to_owned(),
        commit: wallet_engine::LEZ_REV.to_owned(),
        guest_path: "lez/programs".to_owned(),
        // One build makes every builtin; each evidence entry names its own .bin.
        bin: "token.bin".to_owned(),
        docker_tag: docker_tag.to_owned(),
        features: Some("programs".to_owned()),
    };
    let work = std::env::temp_dir().join("logos-kit-verify-builtins");
    let out = verify::build(&source, &work)?;
    let today = today();
    let mut evidence = Vec::new();
    for (name, _, compiled) in verify::builtins() {
        let built = verify::image_in(&out, &format!("{name}.bin"))?;
        ensure!(
            built == compiled,
            "builtin {name}: source builds {} but the pinned artifact is {}",
            verify::image_hex(&built),
            verify::image_hex(&compiled)
        );
        eprintln!("{name}: reproduced {}", verify::image_hex(&built));
        evidence.push(verify::BuiltinEvidence {
            name: name.to_owned(),
            image_id: verify::image_hex(&built),
            source: verify::Source {
                bin: format!("{name}.bin"),
                ..source.clone()
            },
            reproduced: today.clone(),
        });
    }
    println!("{}", serde_json::to_string_pretty(&evidence)?);
    Ok(())
}

fn today() -> String {
    testimonial::iso(testimonial::now_ms())[..10].to_owned()
}

/// The program account a command names, else the registry's for the zone.
fn testimonial_program(session: &Session, program: Option<&str>) -> Result<AccountId> {
    let program = match program {
        Some(p) => p.to_owned(),
        None => testimonial::default_program(&session.zone().chain).with_context(|| {
            format!(
                "no testimonial program is known for {}; pass --program",
                session.zone().chain
            )
        })?,
    };
    wallet_engine::decode::account_id(&program)
}

/// Untrusted text for the terminal: control characters (except newline)
/// escaped, so a stored string can't drive the terminal.
fn shown(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_control() && c != '\n' {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

fn git(args: &[&str]) -> Result<String> {
    let out = std::process::Command::new("git").args(args).output()?;
    ensure!(out.status.success(), "git {} failed", args.join(" "));
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

async fn testimonial_cmd(cli: &Cli, cmd: &TestimonialCmd) -> Result<()> {
    match cmd {
        TestimonialCmd::Post {
            from,
            text,
            username,
            program,
            submission,
        } => {
            let intent = Intent::Testimonial {
                from: from.clone(),
                program: program.clone(),
                submission: submission.clone(),
                username: username.clone(),
                text: text.clone(),
            };
            transact(cli, intent, None).await
        }
        TestimonialCmd::CallArgs {
            program,
            from,
            page,
            text,
            submission,
        } => {
            let call = testimonial::post_call(
                program.parse()?,
                from.parse()?,
                submission,
                *page,
                None,
                text,
                testimonial::now_ms(),
            )?;
            let Intent::Call { accounts, data, .. } = call else {
                unreachable!("post_call builds a call");
            };
            for a in accounts {
                let signer = if a.signer { ":signer" } else { "" };
                println!("--account={}{signer}", a.account);
            }
            println!("--data={data}");
            Ok(())
        }
        TestimonialCmd::List {
            program,
            submission,
        } => {
            let (mut session, _) = open(cli).await?;
            session.connect().await?;
            let program = testimonial_program(&session, program.as_deref())?;
            let core = session.core().context("not connected")?;
            testimonial::check_program(core, program).await?;
            let pages = testimonial::pages(core, program, submission).await?;
            let mut rows = Vec::new();
            for author in pages.iter().flat_map(|p| &p.authors) {
                let author = AccountId::new(*author);
                if let Some(t) = testimonial::record(core, program, submission, author).await? {
                    rows.push(serde_json::json!({
                        "author": author.to_string(), "username": t.username,
                        "text": t.text, "timestampMs": t.timestamp_ms,
                        "time": testimonial::iso(t.timestamp_ms),
                    }));
                }
            }
            print(cli, &serde_json::Value::Array(rows.clone()), || {
                println!("{submission}: {} testimonial(s)", rows.len());
                for (i, r) in rows.iter().enumerate() {
                    println!(
                        "#{i} {} {} ({})\n    {}",
                        r["time"].as_str().unwrap_or(""),
                        r["author"].as_str().unwrap_or(""),
                        shown(r["username"].as_str().unwrap_or("no name")),
                        shown(r["text"].as_str().unwrap_or("")).replace('\n', "\n    ")
                    );
                }
            });
            session.lock()
        }
        TestimonialCmd::Evidence {
            programs,
            submission,
            snapshot,
            cache,
        } => {
            let (mut session, _) = open(cli).await?;
            session.connect().await?;
            let programs = if programs.is_empty() {
                vec![testimonial_program(&session, None)?]
            } else {
                programs
                    .iter()
                    .map(|p| wallet_engine::decode::account_id(p))
                    .collect::<Result<Vec<_>>>()?
            };
            let core = session.core().context("not connected")?;
            let cache = cache.clone().or_else(|| {
                snapshot
                    .as_ref()
                    .map(|d| d.join(format!(".blocks-{}.json", session.zone().id)))
            });
            let evidence =
                testimonial::evidence(core, &programs, submission, cache.as_deref()).await?;
            let value = serde_json::to_value(&evidence)?;
            if let Some(dir) = snapshot {
                std::fs::create_dir_all(dir)?;
                let name = format!("{}-{}.json", submission.replace('/', "_"), today());
                let path = dir.join(name);
                std::fs::write(&path, serde_json::to_vec_pretty(&value)?)?;
                eprintln!("snapshot {}", path.display());
            }
            print(cli, &value, || {
                println!(
                    "{}: {} distinct author(s), tip block {}",
                    evidence.submission, evidence.distinct_authors, evidence.tip.block
                );
                println!("month     new authors");
                for m in &evidence.months {
                    println!("{}   {:>5}", m.month, m.count);
                }
                for p in &evidence.programs {
                    if !p.consistent {
                        println!(
                            "WARNING: {}'s monthly tally differs from its records",
                            p.account
                        );
                    }
                }
                let t = &evidence.target;
                println!(
                    "{} of {} author(s) had their own transactions before posting (prior activity)",
                    t.qualified, evidence.distinct_authors
                );
                println!(
                    "target, qualified authors only: {} total ({}), {} per month over 2 consecutive months ({}) → {}",
                    t.total,
                    if t.qualified_total_met {
                        "met"
                    } else {
                        "not yet"
                    },
                    t.per_month,
                    if t.qualified_months_met {
                        "met"
                    } else {
                        "not yet"
                    },
                    if t.met { "MET" } else { "not met" }
                );
            });
            session.lock()
        }
        TestimonialCmd::Build {
            commit,
            repo_url,
            docker_tag,
            out,
        } => {
            let root = git(&["rev-parse", "--show-toplevel"])?;
            let commit = match commit {
                Some(c) => git(&["rev-parse", &format!("{c}^{{commit}}")])?,
                None => git(&["rev-parse", "HEAD"])?,
            };
            let source = verify::Source {
                repo: root,
                commit,
                guest_path: "programs/testimonial/methods/guest".to_owned(),
                bin: "testimonial.bin".to_owned(),
                docker_tag: docker_tag.clone(),
                features: None,
            };
            // What gets built is the commit; it must be public for anyone
            // to reproduce the image build.json records.
            if git(&["branch", "-r", "--contains", &source.commit])?.is_empty() {
                eprintln!(
                    "warning: {} is on no remote branch yet; push it to {repo_url} before publishing build.json",
                    &source.commit[..12]
                );
            }
            let work = std::env::temp_dir().join("logos-kit-build-testimonial");
            eprintln!(
                "building testimonial @ {} in docker {docker_tag} …",
                &source.commit[..12]
            );
            let dir = verify::build(&source, &work)?;
            let bin = std::fs::read(dir.join(&source.bin))?;
            let image = verify::image_hex(&testimonial::image_of(&bin)?);
            let build = testimonial::Build {
                image_id: image.clone(),
                source: verify::Source {
                    repo: repo_url.clone(),
                    ..source
                },
                built: today(),
            };
            std::fs::create_dir_all(out)?;
            std::fs::write(out.join("testimonial.bin"), &bin)?;
            std::fs::write(
                out.join("build.json"),
                format!("{}\n", serde_json::to_string_pretty(&build)?),
            )?;
            print(cli, &serde_json::to_value(&build)?, || {
                println!("image {image}\nwrote {}", out.display());
            });
            Ok(())
        }
        TestimonialCmd::Deploy {
            payer,
            upgradeable,
            bin,
        } => {
            let immutable = !upgradeable;
            let elf = std::fs::read(bin).with_context(|| format!("reading {}", bin.display()))?;
            let image = verify::image_hex(&testimonial::image_of(&elf)?);
            let pinned = testimonial::build()
                .context("no build.json compiled in; run `testimonial build` and rebuild")?;
            ensure!(
                image == pinned.image_id,
                "{} is image {image}, but this wallet knows the testimonial as {}",
                bin.display(),
                pinned.image_id
            );
            let (mut session, _) = open(cli).await?;
            let payer = wallet_engine::decode::account_id(payer)?;
            if !confirm(
                cli,
                &format!(
                    "Deploy the testimonial program ({}) to {}, paid by {payer}?",
                    if immutable {
                        "immutable"
                    } else {
                        "upgradeable"
                    },
                    session.zone().id
                ),
            )? {
                bail!("cancelled");
            }
            let program = session.deploy_program(elf, payer, immutable).await?;
            let entry = serde_json::json!({
                "name": "testimonial", "chain": session.zone().chain,
                "account": program.to_string(), "imageId": image,
                "source": pinned.source,
            });
            print(cli, &entry, || {
                println!("deployed at {program}");
                println!(
                    "registry entry:\n{}",
                    serde_json::to_string_pretty(&entry).unwrap_or_default()
                );
            });
            session.lock()
        }
    }
}

/// Rebuild one program from its source and compare with what's deployed.
async fn verify_program(cli: &Cli, account: &str, flags: verify::Source) -> Result<()> {
    let source = if flags.repo.is_empty() {
        verify::registry_source(account)
            .map(|(_, s)| s)
            .with_context(|| {
                format!("{account} isn't in the registry; pass --repo --commit --guest-path --bin")
            })?
    } else {
        ensure!(
            !flags.commit.is_empty() && !flags.guest_path.is_empty() && !flags.bin.is_empty(),
            "--repo needs --commit, --guest-path and --bin"
        );
        flags
    };
    let (mut session, _) = open(cli).await?;
    let zone = session.zone().id.clone();
    session.connect().await?;
    let program = wallet_engine::decode::account_id(account)?;
    let header = {
        let core = session.core().context("not connected")?;
        verify::read_header(core, program)
            .await?
            .with_context(|| format!("no program is deployed at {account}"))?
    };
    let work = std::env::temp_dir().join(format!("logos-kit-verify-{account}"));
    eprintln!(
        "building {} @ {} in docker {} …",
        source.repo,
        &source.commit[..source.commit.len().min(12)],
        source.docker_tag
    );
    let s2 = source.clone();
    let built = tokio::task::spawn_blocking(move || verify::build_image_id(&s2, &work)).await??;
    let (live, built_hex) = (
        verify::image_hex(&header.image_id),
        verify::image_hex(&built),
    );
    let matched = built == header.image_id;
    if matched {
        verify::save_cache(
            session.data_dir().root(),
            verify::Verified {
                zone,
                account: account.to_owned(),
                image_id: live.clone(),
                source,
            },
        )?;
    }
    let result = serde_json::json!({
        "account": account, "deployed": live, "built": built_hex,
        "status": if matched { "verified_local" } else { "mismatch" },
        "immutable": header.immutable,
    });
    print(cli, &result, || {
        println!("deployed {live}\nbuilt    {built_hex}");
        println!(
            "{}",
            if matched {
                "verified: the deployed program is this source"
            } else {
                "MISMATCH: the deployed program is not this source"
            }
        );
    });
    session.lock()?;
    ensure!(matched, "source doesn't match the deployed program");
    Ok(())
}

async fn run(cli: Cli) -> Result<()> {
    match &cli.command {
        Command::Init => {
            let data = data_dir(&cli)?;
            let zone = zone(&cli, &data)?;
            let pw = new_password()?;
            let (session, phrase) = Session::create(data, &pw, zone, KdfCost::DEFAULT)?;
            print(&cli, &serde_json::json!({ "created": true }), || {
                println!(
                    "Recovery phrase (write it down; it is shown once):\n\n  {}\n",
                    *phrase
                );
            });
            if cli.json {
                // Scripts need the phrase to test restore; humans saw it above.
                eprintln!("{}", *phrase);
            }
            session.lock()
        }
        Command::Restore {
            from_date,
            from_genesis,
        } => {
            let data = data_dir(&cli)?;
            let zone = zone(&cli, &data)?;
            let phrase = match std::env::var("LOGOS_KIT_PHRASE") {
                Ok(p) => p,
                Err(_) => rpassword::prompt_password("Recovery phrase: ")?,
            };
            let birthday = match (from_date, from_genesis) {
                (Some(d), _) => Birthday::At(date_ms(d)?),
                (None, true) => Birthday::Genesis,
                (None, false) => bail!(
                    "say when the wallet was first used: --from-date YYYY-MM-DD, or --from-genesis"
                ),
            };
            let pw = new_password()?;
            Session::restore(data, &pw, &phrase, birthday, zone, KdfCost::DEFAULT)?.lock()?;
            println!("Restored. Run `logos-kit sync` to find your accounts.");
            Ok(())
        }
        Command::Account(cmd) => {
            let (mut session, _) = open(&cli).await?;
            match cmd {
                AccountCmd::List => {
                    let accounts = session.accounts()?;
                    print(&cli, &serde_json::to_value(&accounts)?, || {
                        for a in &accounts {
                            println!(
                                "{:<8} {:<46} {:<8} {}",
                                format!("{:?}", a.kind).to_lowercase(),
                                a.account_id,
                                a.path.as_deref().unwrap_or("-"),
                                a.label.as_deref().unwrap_or("")
                            );
                        }
                    });
                }
                AccountCmd::New { private } => {
                    let kind = if *private {
                        AccountKind::Private
                    } else {
                        AccountKind::Public
                    };
                    let a = session.new_named_account(kind)?;
                    print(&cli, &serde_json::to_value(&a)?, || {
                        println!("{}", a.account_id);
                    });
                }
                AccountCmd::Label { account, name } => {
                    session.set_label(account, name.as_deref())?;
                }
                AccountCmd::Keys { account } => {
                    let (npk, vpk) = session.receive_keys(account)?;
                    print(&cli, &serde_json::json!({ "npk": npk, "vpk": vpk }), || {
                        println!("{npk}\n{vpk}");
                    });
                }
                AccountCmd::Import => {
                    let key = match std::env::var("LOGOS_KIT_IMPORT_KEY") {
                        Ok(k) => k,
                        Err(_) => rpassword::prompt_password("Private key (hex): ")?,
                    };
                    let a = session.import_public_key(&key)?;
                    print(&cli, &serde_json::to_value(&a)?, || {
                        println!("{}", a.account_id);
                    });
                }
            }
            session.lock()
        }
        Command::Balance { account, token } => {
            let (mut session, _) = open(&cli).await?;
            let balance = session.balance_of(account, token.as_deref()).await?;
            print(
                &cli,
                &serde_json::json!({ "balance": balance.to_string() }),
                || {
                    if token.is_some() {
                        println!("{balance}");
                    } else {
                        println!("{} LGO", lgo(balance));
                    }
                },
            );
            session.lock()
        }
        Command::Sync => {
            let (mut session, _) = open(&cli).await?;
            struct Progress;
            impl wallet::sync_observer::SyncObserver for Progress {}
            let tip = session.sync(&mut Progress).await?;
            print(&cli, &serde_json::json!({ "tip": tip }), || {
                println!("synced to block {tip}");
            });
            session.lock()
        }
        Command::Send(args) => transact(&cli, transfer(args)?, None).await,
        Command::Shield(args) => transact(&cli, transfer(args)?, Some(Route::Shield)).await,
        Command::Deshield(args) => transact(&cli, transfer(args)?, Some(Route::Unshield)).await,
        Command::Token(cmd) => token(&cli, cmd).await,
        Command::Testimonial(cmd) => testimonial_cmd(&cli, cmd).await,
        Command::Faucet {
            account,
            via,
            url,
            key_env,
            drop,
        } => {
            faucet(
                &cli,
                account,
                via.as_deref(),
                url.as_deref(),
                key_env.as_deref(),
                *drop,
            )
            .await
        }
        Command::Programs => {
            let (session, _) = open(&cli).await?;
            let named = session.named_programs();
            print(&cli, &serde_json::to_value(&named)?, || {
                if named.is_empty() {
                    println!("no named programs on {}", session.zone().id);
                }
                for p in &named {
                    println!("{}  {}", p.account, p.name);
                }
            });
            session.lock()
        }
        Command::Program {
            account,
            name,
            forget,
        } => {
            let (mut session, _) = open(&cli).await?;
            if let Some(n) = name {
                session.name_program(account, n)?;
            } else if *forget {
                session.forget_program(account)?;
            }
            let zone = session.zone().id.clone();
            let cache = verify::load_cache(session.data_dir().root());
            session.connect().await?;
            let core = session.core().context("not connected")?;
            // A builtin may be named instead of its address.
            let id = match verify::builtins()
                .into_iter()
                .find(|(n, _, _)| n == account)
            {
                Some((_, id, _)) => id,
                None => wallet_engine::decode::account_id(account)?,
            };
            let named = session.named_programs();
            let check = verify::check_cached(core, id, &zone, &cache, &named).await?;
            print(&cli, &serde_json::to_value(&check)?, || {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&check).unwrap_or_default()
                );
            });
            session.lock()
        }
        Command::VerifyProgram {
            account,
            builtins,
            repo,
            commit,
            guest_path,
            bin,
            features,
            docker_tag,
        } => {
            let flags = verify::Source {
                repo: repo.clone().unwrap_or_default(),
                commit: commit.clone().unwrap_or_default(),
                guest_path: guest_path.clone().unwrap_or_default(),
                bin: bin.clone().unwrap_or_default(),
                docker_tag: docker_tag.clone(),
                features: features.clone(),
            };
            if *builtins {
                verify_builtins(repo.as_deref(), docker_tag)
            } else {
                let account = account
                    .as_deref()
                    .context("give a program account (or --builtins)")?;
                verify_program(&cli, account, flags).await
            }
        }
        Command::Call {
            from,
            program,
            accounts,
            data,
        } => {
            let accounts = accounts
                .iter()
                .map(|a| call_account(a))
                .collect::<Result<Vec<_>>>()?;
            transact(
                &cli,
                Intent::Call {
                    from: from.clone(),
                    program: program.clone(),
                    accounts,
                    data: data.clone(),
                },
                None,
            )
            .await
        }
        Command::Backup(BackupCmd::Export { file }) => {
            let (session, _) = open(&cli).await?;
            let bundle = session.export_backup()?;
            write_private(file, &bundle)?;
            println!("wrote an encrypted backup to {}", file.display());
            session.lock()
        }
        Command::Backup(BackupCmd::Import { file }) => {
            let data = data_dir(&cli)?;
            let zone = zone(&cli, &data)?;
            let bundle = std::fs::read(file).with_context(|| file.display().to_string())?;
            let written = data.import_backup(&bundle)?;
            let pw = password("Password of the backup: ")?;
            match Session::unlock(data.clone(), &pw, zone) {
                Ok(s) => {
                    s.lock()?;
                    println!("Restored from {}.", file.display());
                    Ok(())
                }
                Err(e) => {
                    // Leave no half-restored wallet behind (only what we wrote).
                    wallet_engine::backup::remove_written(data.root(), &written);
                    Err(e.context("the backup didn't open with that password"))
                }
            }
        }
        Command::Status => {
            let (session, _) = open(&cli).await?;
            let status = session.status()?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            session.lock()
        }
        Command::Zones => {
            let zones = data_dir(&cli)?.zones()?;
            println!("{}", serde_json::to_string_pretty(&zones)?);
            Ok(())
        }
        Command::Reveal => {
            let (mut session, pw) = open(&cli).await?;
            let again = if noninteractive().is_some() {
                pw
            } else {
                rpassword::prompt_password("Password again to show the phrase: ")?
            };
            let phrase = session.reveal_phrase(&again)?;
            println!("{}", *phrase);
            session.lock()
        }
        Command::Password => {
            let (mut session, pw) = open(&cli).await?;
            let new = new_password()?;
            session.change_password(&pw, &new)?;
            session.lock()
        }
        Command::AutoLock { seconds } => {
            let (mut session, _) = open(&cli).await?;
            session.set_auto_lock(*seconds)?;
            session.lock()
        }
    }
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    wallet_engine::proving::set_low_memory(cli.low_memory);
    let json = cli.json;
    if let Err(e) = run(cli).await {
        let code = code_of(&e) as i64;
        if json {
            println!(
                "{}",
                serde_json::json!({ "error": { "code": code, "message": format!("{e:#}") } })
            );
        } else {
            eprintln!("error: {e:#}");
        }
        std::process::exit(1);
    }
}
