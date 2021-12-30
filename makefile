build : 
	cargo +nightly build  --release --target=x86_64-unknown-linux-musl --out-dir=./ -Z unstable-options
strip : 
	strip ./proxy
upx : 
	upx --best --lzma ./proxy
all : build strip upx

docker : all
	docker build -t yusongwang:eth-proxy:v$(cat Cargo.toml | grep "version" | head -n 1 | sed 's/=/\n/g' | sed '1d' | sed 's/"/\n/g' | sed '1d' | sed '2d') . && docker push yusongwang:eth-proxy:v$(cat Cargo.toml | grep "version" | head -n 1 | sed 's/=/\n/g' | sed '1d' | sed 's/"/\n/g' | sed '1d' | sed '2d')