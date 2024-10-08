use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use futures::stream::{self, StreamExt};
use indicatif::{ProgressBar, ProgressStyle};
use rand::seq::SliceRandom;
use rand::{thread_rng, Rng};
use reqwest::Client;
use ruint::aliases::U256;
use serde::{Deserialize, Serialize};
use serde_json::json;
use statrs::statistics::Statistics;
use tokio::sync::Mutex;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::RetryIf;
use world_tree::tree::Hash;

macro_rules! hash {
    ($($hash:expr),*) => {
        $(
            Hash::from_be_bytes(hex_literal::hex!($hash))
        )*
    };
}

const IDENTITIES: &[Hash] = &[
    hash!("0e2f1b1fef632f103fc017237a0cd17862bf814727a94f2e97d30d1289108f7c"),
    hash!("2f66b003f78654c802c42cf4d55711f9f988fb858f799f9b22cd02b4122b4f31"),
    hash!("08edcc153c8f9aaf41f2cf779a3c99db2e6306fae44fa5d62230e44c40059d83"),
    hash!("182fec0098c78d30b8a742795d04b6e53c0bfd9dbee3424e04eb4ca2aa65591e"),
    hash!("0a0a2b2b9871b9c03114217d120f6edf447df036dd44745106ce5fdb2d2a8c41"),
    hash!("14bb0b38c2912ba87046581c3464746f88855ac0fdad761338ba22b7d8f170ed"),
    hash!("049b1d83e9e2973b0d405e5b42b8ebf04a1ae0333a41e7e87d8bfa26c51004fb"),
    hash!("15adba9161cc151ee117128f117a654d4f88bb59096966b46dd6a919ff4ad290"),
    hash!("1dea572cf723118501705f6f6fa063933c2db3979e0780faadb98e956b8710d4"),
    hash!("1d753cf0961002315b2eb6f7d55c778f803f3e68773004585f00aab1c6a76f82"),
    hash!("28ec3e6d520c847c0d2630f9c7857e9056ebfb2b59ca1b6dd455880cd640b665"),
    hash!("173440e466f75a3a7e7bd791dadf0f596cbba06aaa7c752a262efa7887d452aa"),
    hash!("2fc4b82595a4400e962377104864c0f317a4dcd3e414d7e3a71f39543fe5a366"),
    hash!("2d926381ee34a61ee684cfe3cb8ebf7ee70b227f676c6f02853e7cb96001a6b1"),
    hash!("05fed32c4515e30a7f3218ac8394f5b4260857a04bd378b9ff70ffa6f13da99a"),
    hash!("14785462f8c89d142514b52cc24cf394ac0f3e59ab626a23f0fe7f0a230cb6e0"),
    hash!("11aea5b6dfb4c447ecf990e3dfaecf9ffa9e8e8efc771e9b86d825e97da36fc7"),
    hash!("2edbd4c6203249360425d553b28efbf38e9c1f0682fdaf510a57c75887963f74"),
    hash!("28dc51bc91e39ac19916c8d0b156a2ebe83d01a0c611f7dce8e43daec4e58726"),
    hash!("1c6d9df3dce59b6caf327c0155386e6940b0f3089246991be1818b6e1b23185d"),
    hash!("0b8bc26695a5dca8220aee3d6456fb552bc50edeefbf49771ac7ec4a91b985be"),
    hash!("03902016020c4bde1ceb6a52fb2c36b4e920e68b03558dec244376c4d935cda9"),
    hash!("136d5e70ea24d4004132f177ec0695e7a294a536c79b71a0abb94b9aa640fa72"),
    hash!("2fa626d773a2e8cac9d20402ea8582ad3c5be793801426ed5af64237b9ef897e"),
    hash!("052d484575283cca14b5f5e21c906f54a485bf0a7552e8f01f4c9f41189a22ac"),
    hash!("0852aa08717b236eff7544a3334d8fa52a3f57bcbf7c53086129d1e1c36e4847"),
    hash!("23f84be094bc3c7150f6549060af7d3d0dbcafbe38824ecfae7bcfbac95b9cee"),
    hash!("04324923cb3625ae55da0a881c6aa8a0d7b9fd7e5914c8ba9968601c25ad0eb0"),
    hash!("2a3c1c00edaebef92455a1d50b843f4d4443d8eb55ada30b2847b6fd0b17177f"),
    hash!("013ed14878c3278bbeea4618c18287480235c885f25bd80712b4a6e4465b2b71"),
    hash!("260b70405a4e893346ebc938cd577bbdb4d5c3b62d1f77c85c67811c67df901f"),
    hash!("14003c157cdb7ca5238ea3f5c02630d898abb8d1730766f1ea69d7f6410e5978"),
    hash!("1fea00de24346b7aaa4a2e2d4e3a327f249a90795257d1873d0430549903708d"),
    hash!("1ecf929697a97c977b58e6d54cc44c2541c2d3c50feac589eb98ff02d1bef683"),
    hash!("0a4a04117357fe952851ea27cdf8f6718aa82bf4a691289cd84a2671851e819b"),
    hash!("17d9361a2881460eadcfafda7fbc39f0cca60ef36842611dfb873f86c082031b"),
    hash!("0fe9164afeda1732bfbdba855dba2c85c1a6f10be3182a6d0a522b16612f0ee0"),
    hash!("1f54ba651d4d8fd3f46d9007b7f119bd8fd8dcd5d9ab506f8b53bf00f23c278b"),
    hash!("19cbbe871782b0bcaf6f4c75694f566b6ad75a4b3cc29131d3d166a0f42e24ad"),
];

#[derive(Debug, Clone, Parser)]
struct Opt {
    #[clap(flatten)]
    common: Common,

    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Args)]
struct Common {
    /// Path to a file containing a list of identity commitments to check
    ///
    /// If not set the default embedded list of identity commitments will be used
    #[clap(short, long)]
    identities_file: Option<PathBuf>,

    /// Endpoint for the World Tree service
    #[clap(
        short,
        long,
        default_value = "https://world-tree.crypto.worldcoin.org/inclusionProof"
    )]
    world_tree_endpoint: String,

    /// Number of concurrent jobs to run
    #[clap(short, long, default_value = "10")]
    jobs: usize,
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    /// Stress tests the service by executing a large number of inclusion proofs
    Stress {
        /// Max number of simultaneous requests
        #[clap(short = 'J', long, default_value = "10")]
        max_simultaneous_requests: usize,

        /// Total number of requests to make
        #[clap(short = 'N', long, default_value = "100")]
        num_requests: usize,

        /// Chain IDs to use (comma-separated)
        #[clap(short, long)]
        chain_ids: Vec<u64>,
    },

    /// Validates consistency between the sequencer and world-tree
    Consistency {
        #[clap(
            short,
            long,
            default_value = "https://signup-orb-ethereum.crypto.worldcoin.org/inclusionProof"
        )]
        sequencer_endpoint: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct InclusionProof {
    root: String,
    proof: serde_json::Value,
}

#[tokio::main]
async fn main() -> eyre::Result<()> {
    let args = Opt::parse();

    let identities = args.common.identities();

    match args.command {
        Command::Stress {
            max_simultaneous_requests,
            num_requests,
            chain_ids,
        } => {
            run_stress_test(
                max_simultaneous_requests,
                num_requests,
                args.common.world_tree_endpoint,
                chain_ids,
                &identities,
            )
            .await
        }
        Command::Consistency { sequencer_endpoint } => {
            run_consistency_check(
                args.common.world_tree_endpoint,
                sequencer_endpoint,
                &identities,
                args.common.jobs,
            )
            .await
        }
    }
}

async fn run_stress_test(
    max_simultaneous_requests: usize,
    num_requests: usize,
    endpoint: String,
    chain_ids: Vec<u64>,
    identities: &[Hash],
) -> eyre::Result<()> {
    let client = Client::new();

    let progress_bar = ProgressBar::new(num_requests as u64);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} [{elapsed_precise}]")?
            .progress_chars("=> "),
    );

    let success_count = Arc::new(AtomicUsize::new(0));
    let failure_count = Arc::new(AtomicUsize::new(0));
    let request_count = Arc::new(AtomicUsize::new(0));
    let times: Arc<std::sync::Mutex<Vec<f64>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    let (tx, mut rx) = tokio::sync::mpsc::channel(max_simultaneous_requests);

    let spawner_task = async {
        for _ in 0..num_requests {
            let mut rng = thread_rng();

            let identity = *identities.choose(&mut rng).unwrap();

            let chain_id_choice = rng.gen::<usize>() % (1 + chain_ids.len());
            let chain_id = if chain_id_choice < chain_ids.len() {
                Some(chain_ids[chain_id_choice])
            } else {
                None
            };

            let endpoint = endpoint.clone();
            let client = client.clone();
            let success_count = success_count.clone();
            let failure_count = failure_count.clone();
            let request_count = request_count.clone();
            let times = times.clone();

            let task = tokio::spawn(async move {
                let request_body = json!({
                    "identityCommitment": identity.to_string(),
                });

                let mut url = endpoint.clone();
                if let Some(chain_id) = chain_id {
                    url = format!("{}?chainId={}", url, chain_id);
                }

                let start = Instant::now();
                let res = client
                    .post(&url)
                    .json(&request_body)
                    .timeout(Duration::from_secs(10))
                    .send()
                    .await;

                let elapsed = start.elapsed().as_secs_f64();
                times.lock().unwrap().push(elapsed);

                match res {
                    Ok(response) if response.status().is_success() => {
                        success_count.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {
                        failure_count.fetch_add(1, Ordering::Relaxed);
                    }
                }

                request_count.fetch_add(1, Ordering::Relaxed);
            });

            tx.send(task).await?;
        }

        drop(tx);

        eyre::Result::<()>::Ok(())
    };

    let consumer_task = async {
        while let Some(task) = rx.recv().await {
            task.await?;

            progress_bar.inc(1);
        }

        eyre::Result::<()>::Ok(())
    };

    let (spawner_task, consumer_task) =
        tokio::join!(spawner_task, consumer_task);

    spawner_task?;
    consumer_task?;

    progress_bar.finish_with_message("Stress test complete");

    // Compute statistics
    let times = times.lock().unwrap();
    let success_rate = success_count.load(Ordering::Relaxed);
    let failure_rate = failure_count.load(Ordering::Relaxed);
    let mean = times.as_slice().mean();
    let min = times.as_slice().min();
    let max = times.as_slice().max();
    let std_dev = times.as_slice().std_dev();

    println!("Success: {}, Failure: {}", success_rate, failure_rate);
    println!(
        "Request times (seconds): avg: {mean:.4}, min: {min:.4}, max: {max:.4}, stddev: {std_dev:.4}",
    );

    Ok(())
}

async fn run_consistency_check(
    world_tree_endpoint: String,
    sequencer_endpoint: String,
    identities: &[Hash],
    jobs: usize,
) -> eyre::Result<()> {
    let client = Client::new();

    let total_progress_bar = ProgressBar::new(identities.len() as u64);
    total_progress_bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} [{elapsed_precise}]")?
            .progress_chars("=> "),
    );

    let matches_count = Arc::new(AtomicU64::new(0));
    let mismatches_count = Arc::new(AtomicU64::new(0));
    let missing_count = Arc::new(AtomicU64::new(0));
    let failures_count = Arc::new(AtomicU64::new(0));

    let missing_identities: Arc<Mutex<Vec<Hash>>> =
        Arc::new(Mutex::new(Vec::new()));
    let failures: Arc<Mutex<Vec<Hash>>> = Arc::new(Mutex::new(Vec::new()));

    let concurrency_limit = jobs; // Adjust as needed

    stream::iter(identities.iter().cloned())
        .map(|identity| {
            let client = client.clone();
            let world_tree_endpoint = world_tree_endpoint.clone();
            let sequencer_endpoint = sequencer_endpoint.clone();
            let matches_count = matches_count.clone();
            let mismatches_count = mismatches_count.clone();
            let missing_count = missing_count.clone();
            let failures_count = failures_count.clone();
            let total_progress_bar = total_progress_bar.clone();
            let missing_identities = missing_identities.clone();
            let failures = failures.clone();

            async move {
                match get_world_tree_inclusion_proof(
                    &client,
                    &world_tree_endpoint,
                    &identity,
                )
                .await
                {
                    Ok(Some(world_tree_response)) => {
                        match get_sequencer_inclusion_proof(
                            &client,
                            &sequencer_endpoint,
                            &identity,
                        )
                        .await
                        {
                            Ok(sequencer_response) => {
                                if world_tree_response.root
                                    == sequencer_response.root
                                    && world_tree_response.proof
                                        == sequencer_response.proof
                                {
                                    matches_count
                                        .fetch_add(1, Ordering::SeqCst);
                                } else {
                                    mismatches_count
                                        .fetch_add(1, Ordering::SeqCst);
                                }
                            }
                            Err(_) => {
                                failures_count.fetch_add(1, Ordering::SeqCst);

                                failures.lock().await.push(identity);
                            }
                        }
                    }
                    Ok(None) => {
                        // Identity missing on world-tree
                        missing_count.fetch_add(1, Ordering::SeqCst);

                        missing_identities.lock().await.push(identity);
                    }
                    Err(_) => {
                        failures_count.fetch_add(1, Ordering::SeqCst);

                        failures.lock().await.push(identity);
                    }
                }
                total_progress_bar.inc(1);
            }
        })
        .buffer_unordered(concurrency_limit)
        .collect::<()>()
        .await;

    total_progress_bar.finish_with_message("Done!");

    println!("Matches: {}", matches_count.load(Ordering::SeqCst));
    println!("Mismatches: {}", mismatches_count.load(Ordering::SeqCst));
    println!("Missing: {}", missing_count.load(Ordering::SeqCst));
    println!("Failures: {}", failures_count.load(Ordering::SeqCst));

    let missing_identities = missing_identities.lock().await;
    if !missing_identities.is_empty() {
        println!("Missing identities:");
        for identity in missing_identities.iter() {
            println!("{}", identity);
        }
    }

    let failures = failures.lock().await;
    if !failures.is_empty() {
        println!("Failures:");
        for identity in failures.iter() {
            println!("{}", identity);
        }
    }

    Ok(())
}

async fn get_world_tree_inclusion_proof(
    client: &Client,
    endpoint: &str,
    identity: &Hash,
) -> Result<Option<InclusionProof>, reqwest::Error> {
    let retry_strategy =
        ExponentialBackoff::from_millis(10).map(jitter).take(5);

    RetryIf::spawn(
        retry_strategy,
        || async {
            let response = client
                .post(endpoint)
                .json(&json!({
                    "identityCommitment": identity.to_string(),
                }))
                .send()
                .await?;

            if response.status() == 404 {
                Ok(None)
            } else {
                let response = response.error_for_status()?;
                let inclusion_proof = response.json().await?;
                Ok(Some(inclusion_proof))
            }
        },
        is_retryable_error,
    )
    .await
}

async fn get_sequencer_inclusion_proof(
    client: &Client,
    endpoint: &str,
    identity: &Hash,
) -> Result<InclusionProof, reqwest::Error> {
    let retry_strategy =
        ExponentialBackoff::from_millis(10).map(jitter).take(5);

    RetryIf::spawn(
        retry_strategy,
        || async {
            let response = client
                .post(endpoint)
                .json(&json!({
                    "identityCommitment": identity.to_string(),
                }))
                .send()
                .await?;

            let response = response.error_for_status()?;
            let inclusion_proof = response.json().await?;
            Ok(inclusion_proof)
        },
        is_retryable_error,
    )
    .await
}

fn is_retryable_error(err: &reqwest::Error) -> bool {
    err.is_timeout() || err.is_connect() || err.is_request()
}

impl Common {
    fn identities(&self) -> Vec<Hash> {
        if let Some(identities_file) = &self.identities_file {
            load_identities_file(identities_file).unwrap()
        } else {
            IDENTITIES.to_vec()
        }
    }
}

fn load_identities_file(
    identities_file: impl AsRef<Path>,
) -> eyre::Result<Vec<Hash>> {
    let identities_file = std::fs::read_to_string(identities_file)?;

    let identities: Vec<Hash> = identities_file
        .lines()
        .map(|line| {
            let identity = U256::from_str_radix(line, 16)?;
            // let identity = line.parse()?;
            Ok(identity)
        })
        .collect::<eyre::Result<_>>()?;

    Ok(identities)
}
