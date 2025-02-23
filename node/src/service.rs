//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.

use futures::FutureExt;
// use hotstuff::import::KnownPeers;
use node_template_runtime::{self, opaque::Block, RuntimeApi};
use sc_client_api::{Backend, BlockBackend};
// use sc_consensus_aura::{ImportQueueParams, StartAuraParams, SlotProportion};
// use sc_consensus_grandpa::{SharedVoterState, ClientForGrandpa};

use sc_consensus_hotstuff::{ImportQueueParams, SlotProportion, StartHotstuffParams};

pub use sc_executor::NativeElseWasmExecutor;
// use sc_network_gossip::GossipEngine;
use sc_service::{error::Error as ServiceError, Configuration, TaskManager};
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
// use sc_network::types::ProtocolName;

// use sp_consensus_aura::sr25519::AuthorityPair as AuraPair;
use sp_consensus_hotstuff::sr25519::AuthorityPair as HotstuffPair;

use std::sync::Arc;

use sc_consensus_grandpafork;
use sc_consensus_hotstuff;

// Our native executor instance.
pub struct ExecutorDispatch;

impl sc_executor::NativeExecutionDispatch for ExecutorDispatch {
	/// Only enable the benchmarking host functions when we actually want to benchmark.
	#[cfg(feature = "runtime-benchmarks")]
	type ExtendHostFunctions = frame_benchmarking::benchmarking::HostFunctions;
	/// Otherwise we only use the default Substrate host functions.
	#[cfg(not(feature = "runtime-benchmarks"))]
	type ExtendHostFunctions = ();

	fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
		node_template_runtime::api::dispatch(method, data)
	}

	fn native_version() -> sc_executor::NativeVersion {
		node_template_runtime::native_version()
	}
}

pub(crate) type FullClient =
	sc_service::TFullClient<Block, RuntimeApi, NativeElseWasmExecutor<ExecutorDispatch>>;
type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;

#[allow(clippy::type_complexity)]
pub fn new_partial(
	config: &Configuration,
) -> Result<
	sc_service::PartialComponents<
		FullClient,
		FullBackend,
		FullSelectChain,
		sc_consensus::DefaultImportQueue<Block, FullClient>,
		sc_transaction_pool::FullPool<Block, FullClient>,
		(
			// sc_consensus_grandpafork::GrandpaBlockImport<
			// 	FullBackend,
			// 	Block,
			// 	FullClient,
			// 	FullSelectChain,
			// >,
			// sc_consensus_grandpafork::LinkHalf<Block, FullClient, FullSelectChain>,
			hotstuff::HotstuffBlockImport<FullBackend, Block, FullClient>,
			hotstuff::LinkHalf<Block, FullClient, FullSelectChain>,
			Option<Telemetry>,
		),
	>,
	ServiceError,
> {
	let telemetry = config
		.telemetry_endpoints
		.clone()
		.filter(|x| !x.is_empty())
		.map(|endpoints| -> Result<_, sc_telemetry::Error> {
			let worker = TelemetryWorker::new(16)?;
			let telemetry = worker.handle().new_telemetry(endpoints);
			Ok((worker, telemetry))
		})
		.transpose()?;

	let executor = sc_service::new_native_or_wasm_executor(config);
	let (client, backend, keystore_container, task_manager) =
		sc_service::new_full_parts::<Block, RuntimeApi, _>(
			config,
			telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
			executor,
		)?;
	let client = Arc::new(client);

	let telemetry = telemetry.map(|(worker, telemetry)| {
		task_manager.spawn_handle().spawn("telemetry", None, worker.run());
		telemetry
	});

	let select_chain = sc_consensus::LongestChain::new(backend.clone());

	let transaction_pool = sc_transaction_pool::BasicPool::new_full(
		config.transaction_pool.clone(),
		config.role.is_authority().into(),
		config.prometheus_registry(),
		task_manager.spawn_essential_handle(),
		client.clone(),
	);

	// let (grandpa_block_import, grandpa_link) = sc_consensus_grandpafork::block_import(
	// 	client.clone(),
	// 	&client,
	// 	select_chain.clone(),
	// 	telemetry.as_ref().map(|x| x.handle()),
	// )?;

	let (grandpa_block_import, grandpa_link) = hotstuff::block_import(client.clone(), &client)?;

	let slot_duration = sc_consensus_hotstuff::slot_duration(&*client)?;

	let import_queue =
		sc_consensus_hotstuff::import_queue::<HotstuffPair, _, _, _, _, _>(ImportQueueParams {
			block_import: grandpa_block_import.clone(),
			justification_import: Some(Box::new(grandpa_block_import.clone())),
			client: client.clone(),
			create_inherent_data_providers: move |_, ()| async move {
				let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

				let slot =
					sp_consensus_hotstuff::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
						*timestamp,
						slot_duration,
					);

				Ok((slot, timestamp))
			},
			spawner: &task_manager.spawn_essential_handle(),
			registry: config.prometheus_registry(),
			check_for_equivocation: Default::default(),
			telemetry: telemetry.as_ref().map(|x| x.handle()),
			compatibility_mode: Default::default(),
		})?;

	Ok(sc_service::PartialComponents {
		client,
		backend,
		task_manager,
		import_queue,
		keystore_container,
		select_chain,
		transaction_pool,
		other: (grandpa_block_import, grandpa_link, telemetry),
	})
}

/// Builds a new service for a full client.
pub fn new_full(config: Configuration) -> Result<TaskManager, ServiceError> {
	let sc_service::PartialComponents {
		client,
		backend,
		mut task_manager,
		import_queue,
		keystore_container,
		select_chain,
		transaction_pool,
		other: (block_import, grandpa_link, mut telemetry),
	} = new_partial(&config)?;
	// let hotstuff_client = client.clone();

	// task_manager.spawn_essential_handle().spawn_blocking(
	// 	"hotstuff-voter-test", None,
	// 	sc_hotstuff_finality::run_hotstuff_voter_test(client.clone())?
	// );

	let mut net_config = sc_network::config::FullNetworkConfiguration::new(&config.network);

	let grandpa_protocol_name = sc_consensus_grandpafork::protocol_standard_name(
		&client.block_hash(0).ok().flatten().expect("Genesis block exists; qed"),
		&config.chain_spec,
	);
	net_config.add_notification_protocol(sc_consensus_grandpafork::grandpa_peers_set_config(
		grandpa_protocol_name.clone(),
	));

	let hotstuff_protocol_name = hotstuff::config::standard_name(
		&client.block_hash(0).ok().flatten().expect("Genesis block exists; qed"),
		&config.chain_spec,
	);

	net_config.add_notification_protocol(hotstuff::config::hotstuff_peers_set_config(
		hotstuff_protocol_name.clone(),
	));

	let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
		sc_service::build_network(sc_service::BuildNetworkParams {
			config: &config,
			net_config,
			client: client.clone(),
			transaction_pool: transaction_pool.clone(),
			spawn_handle: task_manager.spawn_handle(),
			import_queue,
			block_announce_validator_builder: None,
			// warp_sync_params: Some(WarpSyncParams::WithProvider(warp_sync)),
			warp_sync_params: None,
		})?;

	if config.offchain_worker.enabled {
		task_manager.spawn_handle().spawn(
			"offchain-workers-runner",
			"offchain-worker",
			sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
				runtime_api_provider: client.clone(),
				is_validator: config.role.is_authority(),
				keystore: Some(keystore_container.keystore()),
				offchain_db: backend.offchain_storage(),
				transaction_pool: Some(OffchainTransactionPoolFactory::new(
					transaction_pool.clone(),
				)),
				network_provider: network.clone(),
				enable_http_requests: true,
				custom_extensions: |_| vec![],
			})
			.run(client.clone(), task_manager.spawn_handle())
			.boxed(),
		);
	}

	let backoff_authoring_blocks: Option<()> = None;

	let role = config.role.clone();
	let force_authoring = config.force_authoring;
	let _name = config.network.node_name.clone();
	let _enable_grandpa = !config.disable_grandpa;
	let prometheus_registry = config.prometheus_registry().cloned();

	let rpc_extensions_builder = {
		let client = client.clone();
		let pool = transaction_pool.clone();

		Box::new(move |deny_unsafe, _| {
			let deps =
				crate::rpc::FullDeps { client: client.clone(), pool: pool.clone(), deny_unsafe };
			crate::rpc::create_full(deps).map_err(Into::into)
		})
	};

	let _rpc_handlers = sc_service::spawn_tasks(sc_service::SpawnTasksParams {
		network: network.clone(),
		client: client.clone(),
		keystore: keystore_container.keystore(),
		task_manager: &mut task_manager,
		transaction_pool: transaction_pool.clone(),
		rpc_builder: rpc_extensions_builder,
		backend,
		system_rpc_tx,
		tx_handler_controller,
		sync_service: sync_service.clone(),
		config,
		telemetry: telemetry.as_mut(),
	})?;

	if role.is_authority() {
		let proposer_factory = sc_basic_authorship::ProposerFactory::new(
			task_manager.spawn_handle(),
			client.clone(),
			transaction_pool.clone(),
			prometheus_registry.as_ref(),
			telemetry.as_ref().map(|x| x.handle()),
		);

		let slot_duration = sc_consensus_hotstuff::slot_duration(&*client)?;

		let hotstuff =
			sc_consensus_hotstuff::start_hotstuff::<HotstuffPair, _, _, _, _, _, _, _, _, _, _>(
				StartHotstuffParams {
					slot_duration,
					client,
					select_chain,
					block_import,
					proposer_factory,
					create_inherent_data_providers: move |_, ()| async move {
						let timestamp = sp_timestamp::InherentDataProvider::from_system_time();

						let slot =
						sp_consensus_hotstuff::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
							*timestamp,
							slot_duration,
						);

						Ok((slot, timestamp))
					},
					force_authoring,
					backoff_authoring_blocks,
					keystore: keystore_container.keystore(),
					sync_oracle: sync_service.clone(),
					justification_sync_link: sync_service.clone(),
					block_proposal_slot_portion: SlotProportion::new(2f32 / 3f32),
					max_block_proposal_slot_portion: None,
					telemetry: telemetry.as_ref().map(|x| x.handle()),
					compatibility_mode: Default::default(),
				},
			)?;

		// the hotstuff authoring task is considered essential, i.e. if it
		// fails we take down the service with it.
		task_manager.spawn_essential_handle().spawn_blocking(
			"hotstuff",
			Some("block-authoring"),
			hotstuff,
		);

		hotstuff::consensus::start_hotstuff(
			network,
			grandpa_link,
			Arc::new(sync_service),
			hotstuff_protocol_name,
			keystore_container.keystore(),
			&task_manager,
		);
	}

	network_starter.start_network();
	Ok(task_manager)
}
