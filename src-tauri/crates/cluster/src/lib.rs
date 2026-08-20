//! Cluster llama.cpp RPC na LAN: 1 host + 1 worker.
//!
//! O host roda `llama-server` com `--rpc`. O worker, depois de um toque de
//! aceite, sobe `ggml-rpc-server` pinado num único device. A descoberta é
//! mDNS; o RPC só escuta depois do aceite.

mod budget;
mod host;
mod http;
mod persist;
mod split;
mod types;
mod worker;

pub use budget::{advertised_bytes, device_pin, gpu_label};
pub use host::{ClusterHost, EngineBusyFn, NotifyFn, PidFn, RpcExeFn, SaveFn};
pub use persist::{ClusterPersist, LastRole, SETTING_KEY, new_instance_id};
pub use split::{SplitPlan, llama_rpc_args, plan_split, tags_compatible};
pub use types::{ClusterRole, ClusterSnapshot, ConnectedView, NodeIdentity, PeerView};
pub use worker::{DEFAULT_RPC_PORT, RpcWorker, WorkerError, worker_args};
