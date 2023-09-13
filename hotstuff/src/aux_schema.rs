
use std::marker::PhantomData;

use sp_blockchain::{Error as ClientError, Result as ClientResult};
use sp_runtime::traits::{Block as BlockT, NumberFor};
use sc_client_api::backend::AuxStore;
use sp_consensus_grandpa::AuthorityList;

use crate::authorities::{SharedAuthoritySet, AuthoritySet};

/// Persistent data kept between runs.
pub(crate) struct PersistentData<Block: BlockT> {
	pub(crate) authority_set: SharedAuthoritySet<Block::Hash, NumberFor<Block>>,
}

/// Load or initialize persistent data from backend.
pub(crate) fn load_persistent<Block: BlockT, B, G>(
	backend: &B,
	genesis_hash: Block::Hash,
	genesis_number: NumberFor<Block>,
	genesis_authorities: G,
) -> ClientResult<PersistentData<Block>>
where
	B: AuxStore,
	G: FnOnce() -> ClientResult<AuthorityList>,
{

    let genesis_authorities = genesis_authorities()?;

	let genesis_set = AuthoritySet::genesis(genesis_authorities)
	.expect("genesis authorities is non-empty; all weights are non-zero; qed.");

    Ok(PersistentData {authority_set: genesis_set.into()})
}