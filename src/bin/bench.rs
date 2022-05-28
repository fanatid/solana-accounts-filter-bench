use {
    anyhow::Result,
    clap::Parser,
    rand::{RngCore, SeedableRng},
    rand_chacha::ChaCha8Rng,
    // rayon::prelude::*,
    rayon::iter::{IntoParallelRefIterator, ParallelIterator},
    serde::{de, Deserialize, Deserializer},
    solana_sdk::{clock::Slot, pubkey::Pubkey},
    std::{
        collections::{BTreeMap, HashSet},
        fs::File,
        io::BufReader,
        path::PathBuf,
        time::{Duration, SystemTime},
    },
};

#[derive(Debug, Parser)]
#[clap(author, version, about)]
struct Args {
    /// Input file with the data
    #[clap(short, long, default_value = "data.json", parse(from_os_str))]
    input: PathBuf,

    /// Seed for PRNG
    #[clap(short, long, default_value_t = 42)]
    seed: u64,

    /// Minimum seconds for bench.
    #[clap(short, long, default_value_t = 30)]
    min_work: u64,
}

impl Args {
    fn load_blocks(&self) -> Result<Blocks> {
        let file = File::open(self.input.clone())?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).map_err(Into::into)
    }

    fn prng(&self) -> PubkeyRng {
        PubkeyRng {
            rng: ChaCha8Rng::seed_from_u64(self.seed),
        }
    }
}

struct PubkeyRng {
    rng: ChaCha8Rng,
}

impl PubkeyRng {
    fn next(&mut self) -> Pubkey {
        let mut bytes = [0u8; 32];
        self.rng.fill_bytes(&mut bytes);
        Pubkey::new_from_array(bytes)
    }

    #[allow(dead_code)]
    fn next2(&mut self, blocks: &Blocks) -> Pubkey {
        if self.rng.next_u32() > u32::MAX / 2 {
            self.next()
        } else {
            loop {
                let index = (self.rng.next_u32() as usize) % blocks.len();
                let block = blocks.values().nth(index).expect("valid index");
                if !block.pubkeys.is_empty() {
                    let index = (self.rng.next_u32() as usize) % block.pubkeys.len();
                    break *block.pubkeys.get(index).expect("valid index");
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct Block {
    // block_time: solana_sdk::clock::UnixTimestamp,
    #[serde(deserialize_with = "Block::load_pubkeys")]
    pubkeys: Vec<Pubkey>,
}

impl Block {
    fn load_pubkeys<'de, D>(deserializer: D) -> Result<Vec<Pubkey>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<String>::deserialize(deserializer)?
            .into_iter()
            .map(|pubkey| pubkey.parse().map_err(de::Error::custom))
            .collect()
    }
}

type Blocks = BTreeMap<Slot, Block>;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    let ts = SystemTime::now();
    let blocks = args.load_blocks()?;
    println!(
        "Total slots: {}, elapsed: {:?}",
        blocks.len(),
        ts.elapsed()?
    );

    let min_work = Duration::from_secs(args.min_work);
    bench_hashset(&blocks, args.prng(), min_work)?;
    bench_hashset_rayon(&blocks, args.prng(), min_work)?;

    Ok(())
}

fn bench_hashset(blocks: &Blocks, mut prng: PubkeyRng, min_work: Duration) -> Result<()> {
    let ts = SystemTime::now();
    let mut set = HashSet::new();
    while set.len() < 1_000_000 {
        set.insert(prng.next());
        // set.insert(prng.next2(blocks));
    }
    let elapsed = ts.elapsed()?;
    println!("Fill HashSet with len {} in: {:?}", set.len(), elapsed);

    let ts = SystemTime::now();
    let mut iters = 0;
    let mut total_ops = 0;
    let mut success = 0;
    while ts.elapsed()? < min_work {
        iters += 1;
        for block in blocks.values() {
            total_ops += block.pubkeys.len();
            for pubkey in block.pubkeys.iter() {
                if set.contains(pubkey) {
                    success += 1;
                }
            }
        }
    }
    let elapsed = ts.elapsed()?;
    println!(
        "Total slots: {}, total ops: {}, iters: {}, elapsed per block: {:?}, per block: {:?}, per pubkey: {:?} (succes: {})",
        blocks.len(),
        total_ops,
        iters,
        elapsed / iters,
        elapsed / iters / blocks.len() as u32,
        elapsed / iters / total_ops as u32,
        success
    );

    Ok(())
}

fn bench_hashset_rayon(blocks: &Blocks, mut prng: PubkeyRng, min_work: Duration) -> Result<()> {
    let ts = SystemTime::now();
    let mut set = HashSet::new();
    while set.len() < 1_000_000 {
        set.insert(prng.next());
        // set.insert(prng.next2(blocks));
    }
    let elapsed = ts.elapsed()?;
    println!("Fill HashSet with len {} in: {:?}", set.len(), elapsed);

    let ts = SystemTime::now();
    let mut iters = 0;
    let mut total_ops = 0;
    let mut success = 0;
    while ts.elapsed()? < min_work {
        iters += 1;
        for block in blocks.values() {
            total_ops += block.pubkeys.len();
            success += block
                .pubkeys
                .par_iter()
                .filter(|pubkey| set.contains(pubkey))
                .count();
        }
    }
    let elapsed = ts.elapsed()?;
    println!(
        "Total slots: {}, total ops: {}, iters: {}, elapsed per blocks: {:?}, per block: {:?}, per pubkey: {:?} (succes: {})",
        blocks.len(),
        total_ops,
        iters,
        elapsed / iters,
        elapsed / iters / blocks.len() as u32,
        elapsed / iters / total_ops as u32,
        success
    );

    Ok(())
}
