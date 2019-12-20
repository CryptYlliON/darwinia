//! Test utilities

use primitives::H256;
use sr_primitives::{testing::Header, traits::IdentityLookup, weights::Weight, Perbill};
use support::{impl_outer_origin, parameter_types};

use crate::*;

/// The AccountId alias in this test module.
pub type AccountId = u64;
pub type BlockNumber = u64;

pub type System = system::Module<Test>;

pub type EthRelay = Module<Test>;

impl_outer_origin! {
	pub enum Origin for Test {}
}

// Workaround for https://github.com/rust-lang/rust/issues/26925 . Remove when sorted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Test;
parameter_types! {
	pub const BlockHashCount: BlockNumber = 250;
	pub const MaximumBlockWeight: Weight = 1024;
	pub const MaximumBlockLength: u32 = 2 * 1024;
	pub const AvailableBlockRatio: Perbill = Perbill::one();
}
impl system::Trait for Test {
	type Origin = Origin;
	type Call = ();
	type Index = u64;
	type BlockNumber = BlockNumber;
	type Hash = H256;
	type Hashing = ::sr_primitives::traits::BlakeTwo256;
	type AccountId = AccountId;
	type Lookup = IdentityLookup<Self::AccountId>;
	type Header = Header;
	type Event = ();
	type BlockHashCount = BlockHashCount;
	type MaximumBlockWeight = MaximumBlockWeight;
	type MaximumBlockLength = MaximumBlockLength;
	type AvailableBlockRatio = AvailableBlockRatio;
	type Version = ();
}

parameter_types! {
//	pub const EthMainet: u64 = 0;
	pub const EthRopsten: u64 = 1;
}

impl Trait for Test {
	type Event = ();
	type EthNetwork = EthRopsten;
}

pub struct ExtBuilder;
impl Default for ExtBuilder {
	fn default() -> Self {
		Self
	}
}
impl ExtBuilder {
	pub fn build(self) -> runtime_io::TestExternalities {
		let t = system::GenesisConfig::default().build_storage::<Test>().unwrap();

		t.into()
	}
}