#!/bin/bash
set -e

echo "开始编译 lensup..."
cargo build

if [ -f target/debug/lensup ]; then
	echo "拷贝 lensup 到 /usr/local/bin（需要 sudo 权限）..."
	sudo cp target/debug/lensup /usr/local/bin/lensup
	echo "已安装到 /usr/local/bin/lensup，可直接 lensup --from ... 使用。"
else
	echo "未找到 target/debug/lensup，编译失败或文件名不符。"
fi
