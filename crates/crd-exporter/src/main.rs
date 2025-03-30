use std::{
    fs::{self},
    path::PathBuf,
};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use externalip_manager_manager::crd::v1alpha1;
use kube::CustomResourceExt;

#[derive(Debug, Clone, PartialEq, Eq, ValueEnum, Default)]
enum ApiVersion {
    #[default]
    V1Alpha1,
}

#[derive(Parser, Debug)]
#[command(version, about)]
struct Arg {
    /// Directory to save the CRDs into
    #[arg()]
    output_dir: PathBuf,
    /// Which version of CRDs to print
    #[arg(long, value_enum, default_value_t = ApiVersion::V1Alpha1)]
    api_version: ApiVersion,
}

fn write(crd: String, dest: PathBuf) -> Result<()> {
    // dirty, dirty hack to fix https://github.com/kube-rs/kube/issues/1680 for now
    let crd = crd.replace("additionalPrinterColumns: []\n    name:", "name:");
    Ok(fs::write(dest, crd)?)
}

fn main() -> Result<()> {
    let args = Arg::parse();

    match args.api_version {
        ApiVersion::V1Alpha1 => write(
            serde_yaml::to_string(&v1alpha1::ClusterExternalIPSource::crd()).unwrap(),
            args.output_dir
                .join("v1alpha1-ClusterExternalIPSource.yaml"),
        ),
    }
}
