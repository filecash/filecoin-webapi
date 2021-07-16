
# filecoin-webapi

## Build

Dependencies:
- rustc
- cargo

Build with cargo

```bash
git clone git@github.com:sbwtw/filecoin-webapi.git
cargo build --release
```

## Run
```bash
export FIC_C2_TASK_NUM=2  # Number of C2 tasks

cd target/release
./filecoin-webapi 0.0.0.0:6006
```
