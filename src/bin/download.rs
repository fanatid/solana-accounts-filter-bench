use {
    anyhow::Result,
    clap::Parser,
    futures::future::try_join_all,
    serde::Serialize,
    solana_cli_config::{Config, CONFIG_FILE},
    solana_client::nonblocking::rpc_client::RpcClient,
    solana_sdk::{
        clock::{Slot, UnixTimestamp},
        commitment_config::CommitmentConfig,
        message::VersionedMessage,
    },
    solana_transaction_status::UiTransactionEncoding,
    std::{
        collections::{BTreeMap, HashSet},
        path::PathBuf,
        sync::Arc,
    },
    tokio::{
        sync::Mutex,
        time::{sleep, Duration},
    },
};

#[derive(Debug, Parser)]
#[clap(author, version, about)]
struct Args {
    /// Optional Json Rpc Url. By default value from `config.yml`.
    #[clap(short, long)]
    rpc: Option<String>,

    /// Optional slot from where collect Pubkeys, backwise. By default latest finalized slot.
    #[clap(short, long)]
    from: Option<Slot>,

    /// Number of concurrent downloads of blocks.
    #[clap(short = 't', long, default_value_t = 3)]
    concurrency: Slot,

    /// Number of seconds for collecting Pubkeys from the slots.
    #[clap(short, long, default_value_t = 900)] // 15min.
    count: UnixTimestamp,

    /// Out file for the data
    #[clap(short, long, default_value = "data.json", parse(from_os_str))]
    out: PathBuf,
}

#[derive(Debug, Serialize)]
struct Block {
    block_time: UnixTimestamp,
    pubkeys: HashSet<String>,
}

struct SlotsInner {
    slots: Vec<Slot>,
    end_slot: Option<Slot>,
}

struct Slots {
    rpc: Arc<RpcClient>,
    inner: Mutex<SlotsInner>,
    block_time_stop: UnixTimestamp,
}

impl Slots {
    fn new(rpc: Arc<RpcClient>, end_slot: Slot, block_time_stop: UnixTimestamp) -> Self {
        Self {
            rpc,
            inner: Mutex::new(SlotsInner {
                slots: vec![],
                end_slot: Some(end_slot),
            }),
            block_time_stop,
        }
    }

    async fn next(&self) -> Result<Option<Slot>> {
        let mut inner = self.inner.lock().await;
        if let Some(slot) = inner.slots.pop() {
            return Ok(Some(slot));
        }

        Ok(match inner.end_slot {
            Some(end_slot) => {
                let start_slot = end_slot - 1_000;
                println!("Request slots [{}, {}]", start_slot, end_slot);

                let mut attempts = 5;
                let slots = loop {
                    match self.rpc.get_blocks(start_slot, Some(end_slot)).await {
                        Ok(slots) => break slots,
                        Err(error) if attempts == 0 => return Err(error.into()),
                        Err(error) => {
                            attempts -= 1;
                            println!("failed to get slots: {:?}", error);
                            sleep(Duration::from_secs(10)).await;
                        }
                    }
                };

                inner.slots = slots;
                inner.end_slot = inner.slots.first().map(|slot| *slot - 1);
                inner.slots.pop()
            }
            None => None,
        })
    }

    async fn remove_by_block_time(&self, slot: Slot, block_time: UnixTimestamp) {
        if block_time < self.block_time_stop {
            let mut inner = self.inner.lock().await;
            inner.slots = inner
                .slots
                .iter()
                .filter(|islot| **islot >= slot)
                .cloned()
                .collect();
            inner.end_slot = None;
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let json_rpc_url = match args.rpc {
        Some(rpc) => rpc,
        None => {
            let config_file = CONFIG_FILE
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("unable to get config file path"))?;
            Config::load(config_file)?.json_rpc_url
        }
    };

    let rpc = Arc::new(RpcClient::new_with_commitment(
        json_rpc_url,
        CommitmentConfig::finalized(),
    ));
    let slot = match args.from {
        Some(slot) => slot,
        None => rpc.get_slot().await?,
    };
    let block_time_start = rpc.get_block_time(slot).await?;

    let slots = Arc::new(Slots::new(
        Arc::clone(&rpc),
        slot,
        block_time_start - args.count,
    ));
    let blocks = Arc::new(Mutex::new(BTreeMap::new()));

    try_join_all((0..args.concurrency).map(|_| {
        let rpc = Arc::clone(&rpc);
        let slots = Arc::clone(&slots);
        let blocks = Arc::clone(&blocks);
        async move {
            while let Some(slot) = slots.next().await? {
                let mut attempts = 5;
                let block = loop {
                    let encoding = UiTransactionEncoding::Base64;
                    match rpc.get_block_with_encoding(slot, encoding).await {
                        Ok(block) => break block,
                        Err(error) if attempts == 0 => return Err(error.into()),
                        Err(error) => {
                            attempts -= 1;
                            println!("failed to get block {}: {:?}", slot, error);
                            sleep(Duration::from_secs(10)).await;
                        }
                    }
                };

                let block_time = match block.block_time {
                    Some(block_time) => block_time,
                    None => continue,
                };
                println!(
                    "Download block {} with time {}, stop time {}, left {}",
                    slot,
                    block_time,
                    slots.block_time_stop,
                    block_time - slots.block_time_stop
                );

                if block_time < slots.block_time_stop {
                    slots.remove_by_block_time(slot, block_time).await;
                    continue;
                }

                // collect accounts
                let pubkeys = block
                    .transactions
                    .iter()
                    .flat_map(|transaction| {
                        transaction
                            .transaction
                            .decode()
                            .map(|transaction| match transaction.message {
                                VersionedMessage::Legacy(message) => message.account_keys,
                                VersionedMessage::V0(message) => message.account_keys,
                            })
                            .unwrap_or_default()
                            .into_iter()
                            .map(|pubkey| pubkey.to_string())
                    })
                    .collect::<HashSet<_>>();

                let mut blocks = blocks.lock().await;
                blocks.insert(
                    slot,
                    Block {
                        block_time,
                        pubkeys,
                    },
                );
            }
            Ok::<(), anyhow::Error>(())
        }
    }))
    .await?;

    tokio::fs::write(args.out, serde_json::to_string(&*blocks.lock().await)?).await?;

    let blocks = Arc::try_unwrap(blocks).expect("one ref").into_inner();
    println!(
        "Total {} blocks, with {} pubkeys",
        blocks.len(),
        blocks
            .into_iter()
            .fold(HashSet::<String>::new(), |mut acc, (_slot, block)| {
                for pubkey in block.pubkeys {
                    acc.insert(pubkey);
                }
                acc
            })
            .len()
    );

    Ok(())
}
