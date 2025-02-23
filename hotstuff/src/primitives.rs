use sp_consensus_hotstuff::AuthorityId;

// define some primitives used in hotstuff
pub type ViewNumber = u64;

// TODO The `AuthorityId` in this context should be reference instead of value?
#[derive(Debug, PartialEq, Eq)]
pub enum HotstuffError {
	// Receive more then one vote from the same authority.
	AuthorityReuse(AuthorityId),

	// The QC without a quorum
	QCRequiresQuorum,

	// Get invalid signature from a authority.
	InvalidSignature(AuthorityId),

	NullSignature,

	UnknownAuthority(AuthorityId),

	// The voter is not in authorities.
	NotAuthority,

	WrongProposer,

	ProposalNotFound,

	InvalidVote,

	InvalidTC,

	FinalizeBlock(String),

	SaveProposal(String),

	Other(String),
}
