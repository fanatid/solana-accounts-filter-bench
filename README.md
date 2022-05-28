## Accounts filter benchmark

Benchmark of `std::collections::HashSet` with 1M Public Keys for checking that set contains Public Keys from the slots.

### Download slots data

First we need download slots and extract Public Keys (last 6 hours for next command):

```
cargo run --bin download --release -- --rpc http://localhost:8899 --concurrency 50 --count 21600 --out data-360min.json
```

### Run benchmark

```
cargo run --bin bench --release -- --input ./data-360min.json
```

```
Total slots: 22821, elapsed: 65.313779378s
Fill HashSet with len 1000000 in: 152.590988ms
Total slots: 22821, total ops: 1634776227, iters: 21, elapsed per block: 1.437992746s, per block: 63.011µs, per pubkey: 0ns (succes: 0)
Fill HashSet with len 1000000 in: 146.742862ms
Total slots: 22821, total ops: 1556929740, iters: 20, elapsed per blocks: 1.50364883s, per block: 65.888µs, per pubkey: 0ns (succes: 0)
```

Second benchmark use `rayon` which looks like do not give any performance improvements.
