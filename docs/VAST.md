# Sweeping on rented cores

The forty-five-language sweep is CPU-bound and embarrassingly parallel across
files. On this laptop a full js sweep takes about ten minutes at six workers; an
80-core box does it in under two. Renting is worth it when the wall-clock win is
real — a phase that needs three full sweeps — and never worth it for one file.

**Budget: about 4 cents per 80-core sweep.** That number is the whole reason the
destroy step is in a trap rather than a line at the end of the script: a box that
outlives its job is the only way this costs more than four cents.

## The recipe

```bash
# 1. the offer: 80 cores, a modern image, storage for the corpus
vastai search offers 'num_gpus=0 cpu_cores_effective>=80 disk_space>=60' --order dph_total --limit 5

# 2. rent it — the create call prints the contract id that the trap destroys
ID=$(vastai create instance <offer_id> --image ubuntu:22.04 --disk 80 --raw \
      | python3 -c 'import json,sys; print(json.load(sys.stdin)["new_contract"])')
echo "$ID"

# 3. the key: this box's public key has to be on the instance before ssh works
vastai ssh-key add ~/.ssh/id_ed25519.pub 2>/dev/null || true

# 4. wait for it to answer, then copy the corpus and the binary
until ssh -o ConnectTimeout=5 -o BatchMode=yes root@<host> -p <port> true 2>/dev/null; do sleep 5; done
rsync -az /home/roni/Roni_workspace/corpus45/ root@<host>:/root/corpus45/

# 5. sweep, then destroy — always, even when the sweep dies
vastai destroy instance "$ID"
```

`tools/vast_sweep.sh` wraps steps 2 and 5 so the destroy cannot be forgotten:

```bash
tools/vast_sweep.sh --disk 80 --image ubuntu:22.04 -- ./run-sweep.sh
```

Rent, run the command after `--`, destroy. The trap covers `EXIT`, `INT`, `TERM`
and `HUP`, and the destroy is retried once because a transient API error must not
leave a box running. The rule is proven, not assumed:

```
$ tools/vast_sweep.sh --self-test
SELF-TEST OK: failed sweep still destroyed instance 12345
```

## What the numbers mean

Use the box for measurements only. A build compiled there links against a
different libc and a different linker, and its fallback counts are not this
laptop's counts — the corpus table in `docs/corpus45-table-2026-11-12.md` was
taken on the box for exactly that reason, and it is a language-coverage map, not
a baseline.

Baselines come from `tools/shard_sweep.py` on this laptop. The box is for going
faster over the same binary, never for deciding what the binary does.
