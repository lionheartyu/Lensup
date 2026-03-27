#!/bin/bash
set -e

mkdir -p ~/.cargo
cat > ~/.cargo/config.toml <<EOF
[source.crates-io]
replace-with = 'ustc'

[source.ustc]
registry = "https://mirrors.ustc.edu.cn/crates.io-index"
EOF

echo "已写入 ~/.cargo/config.toml："
cat ~/.cargo/config.toml
