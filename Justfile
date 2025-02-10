shared_args := "--release --all-features --workspace --exclude externalip-manager-crd-exporter"

lint:
    cargo clippy
format:
    cargo fmt

build: crds
    cargo build {{ shared_args }}
build-cross target:
    cross build {{ shared_args }} --target {{ target }}

test:
    cargo test {{ shared_args }}
test-cross target:
    cross test {{ shared_args }} --target {{ target }}

docker tag: build
    docker buildx build --tag {{ tag }} .

crds: crds-v1alpha1
crds-v1alpha1:
    cargo run -p externalip-manager-crd-exporter crds/v1alpha1
