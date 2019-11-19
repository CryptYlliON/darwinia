#![cfg_attr(not(feature = "std"), no_std)]

use codec::{Codec, Decode, Encode};
#[cfg(not(feature = "std"))]
use rstd::borrow::ToOwned;
use rstd::{cmp, fmt::Debug, prelude::*, result};
#[cfg(feature = "std")]
use sr_primitives::traits::One;
use sr_primitives::{
	traits::{
		Bounded, CheckedAdd, CheckedSub, MaybeSerializeDeserialize, Member, Saturating, SimpleArithmetic, StaticLookup,
		Zero,
	},
	RuntimeDebug,
};
use srml_support::{
	decl_event, decl_module, decl_storage,
	dispatch::Result,
	traits::{
		Currency, ExistenceRequirement, Imbalance, OnUnbalanced, SignedImbalance, UpdateBalanceOutcome, WithdrawReason,
		WithdrawReasons,
	},
	Parameter, StorageMap, StorageValue,
};
use system::ensure_signed;

use darwinia_support::{
	traits::LockableCurrency,
	types::{CompositeLock, Lock},
};
use imbalance::{NegativeImbalance, PositiveImbalance};

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod imbalance;

/// Struct to encode the vesting schedule of an individual account.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Encode, Decode, Default, RuntimeDebug)]
pub struct VestingSchedule<Balance> {
	/// Locked amount at genesis.
	pub offset: Balance,
	/// Amount that gets unlocked every block from genesis.
	pub per_block: Balance,
}

impl<Balance: SimpleArithmetic + Copy> VestingSchedule<Balance> {
	/// Amount locked at block `n`.
	pub fn locked_at<BlockNumber>(&self, n: BlockNumber) -> Balance
	where
		Balance: From<BlockNumber>,
	{
		if let Some(x) = Balance::from(n).checked_mul(&self.per_block) {
			self.offset.max(x) - x
		} else {
			Zero::zero()
		}
	}
}

pub trait Trait: timestamp::Trait {
	type Balance: Parameter
		+ Member
		+ SimpleArithmetic
		+ Codec
		+ Default
		+ Copy
		+ MaybeSerializeDeserialize
		+ From<Self::BlockNumber>;

	type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;

	// kton
	type OnMinted: OnUnbalanced<PositiveImbalance<Self>>;
	type OnRemoval: OnUnbalanced<NegativeImbalance<Self>>;
}

decl_event!(
    pub enum Event<T> where
        < T as system::Trait>::AccountId,
        < T as Trait>::Balance,
    {
        /// Transfer succeeded (from, to, value, fees).
        TokenTransfer(AccountId, AccountId, Balance),
    }
);

decl_storage! {
	trait Store for Module<T: Trait> as Kton {

		/// For Currency and LockableCurrency Trait
		/// The total `units issued in the system.
		// like `existential_deposit`, but always set to 0
		pub MinimumBalance get(minimum_balance): T::Balance = 0.into();

		pub TotalIssuance get(total_issuance) build(|config: &GenesisConfig<T>| {
			config.balances.iter().fold(Zero::zero(), |acc: T::Balance, &(_, n)| acc + n)
		}): T::Balance;

		pub FreeBalance get(free_balance) build(|config: &GenesisConfig<T>| config.balances.clone()):
			map T::AccountId => T::Balance;

		pub ReservedBalance get(reserved_balance): map T::AccountId => T::Balance;

		pub Locks get(locks): map T::AccountId => BalanceLocks<T::Balance, T::Moment>;

		pub TotalLock get(total_lock): T::Balance;

		pub Vesting get(vesting) build(|config: &GenesisConfig<T>| {
			config.vesting.iter().filter_map(|&(ref who, begin, length)| {
				let begin = <T::Balance as From<T::BlockNumber>>::from(begin);
				let length = <T::Balance as From<T::BlockNumber>>::from(length);

				config.balances.iter()
					.find(|&&(ref w, _)| w == who)
					.map(|&(_, balance)| {
						// <= begin it should be >= balance
						// >= begin+length it should be <= 0

						let per_block = balance / length.max(One::one());
						let offset = begin * per_block + balance;

						(who.clone(), VestingSchedule { offset, per_block })
					})
			}).collect::<Vec<_>>()
		}): map T::AccountId => Option<VestingSchedule<T::Balance>>;
	}
	add_extra_genesis {
		config(balances): Vec<(T::AccountId, T::Balance)>;
		config(vesting): Vec<(T::AccountId, T::BlockNumber, T::BlockNumber)>;		// begin, length
	}
}

decl_module! {
	pub struct Module<T: Trait> for enum Call where origin: T::Origin {
		fn deposit_event() = default;

		pub fn transfer(origin,
			dest: <T::Lookup as StaticLookup>::Source,
			#[compact] value: T::Balance
		) {
			let transactor = ensure_signed(origin)?;
			let dest = T::Lookup::lookup(dest)?;

			<Self as Currency<_>>::transfer(&transactor, &dest, value, ExistenceRequirement::AllowDeath)?;
		}
	}
}

impl<T: Trait> Module<T> {
	pub fn vesting_balance(who: &T::AccountId) -> T::Balance {
		if let Some(v) = Self::vesting(who) {
			Self::free_balance(who).min(v.locked_at::<T::BlockNumber>(<system::Module<T>>::block_number()))
		} else {
			Zero::zero()
		}
	}

	// PRIVATE MUTABLE
	// NOTE: different from balances module
	fn set_free_balance(who: &T::AccountId, balance: T::Balance) -> UpdateBalanceOutcome {
		//TODO: check the value of balance, but no ensure!(...)
		<FreeBalance<T>>::insert(who, balance);
		UpdateBalanceOutcome::Updated
	}

	fn set_reserved_balance(who: &T::AccountId, balance: T::Balance) -> UpdateBalanceOutcome {
		<ReservedBalance<T>>::insert(who, balance);
		UpdateBalanceOutcome::Updated
	}
}

impl<T: Trait> Currency<T::AccountId> for Module<T> {
	type Balance = T::Balance;
	type PositiveImbalance = PositiveImbalance<T>;
	type NegativeImbalance = NegativeImbalance<T>;

	fn total_balance(who: &T::AccountId) -> Self::Balance {
		Self::free_balance(who) + Self::reserved_balance(who)
	}

	fn can_slash(who: &T::AccountId, value: Self::Balance) -> bool {
		Self::free_balance(who) >= value
	}

	fn total_issuance() -> Self::Balance {
		Self::total_issuance()
	}

	fn minimum_balance() -> Self::Balance {
		Self::minimum_balance()
	}

	// TODO: ready for hacking
	fn burn(mut amount: Self::Balance) -> Self::PositiveImbalance {
		<TotalIssuance<T>>::mutate(|issued| {
			issued.checked_sub(&amount).unwrap_or_else(|| {
				amount = *issued;
				Zero::zero()
			})
		});
		PositiveImbalance::new(amount)
	}

	// TODO: ready for hacking
	fn issue(mut amount: Self::Balance) -> Self::NegativeImbalance {
		<TotalIssuance<T>>::mutate(|issued| {
			*issued = issued.checked_add(&amount).unwrap_or_else(|| {
				amount = Self::Balance::max_value() - *issued;
				Self::Balance::max_value()
			})
		});
		NegativeImbalance::new(amount)
	}

	fn free_balance(who: &T::AccountId) -> Self::Balance {
		<FreeBalance<T>>::get(who)
	}

	fn ensure_can_withdraw(
		who: &T::AccountId,
		_amount: T::Balance,
		reasons: WithdrawReasons,
		new_balance: T::Balance,
	) -> Result {
		if reasons.intersects(WithdrawReason::Reserve | WithdrawReason::Transfer)
			&& Self::vesting_balance(who) > new_balance
		{
			Err("vesting balance too high to send value")
		} else {
			if Self::can_withdraw(who, reasons, new_balance) {
				Ok(())
			} else {
				Err("account liquidity restrictions prevent withdrawal")
			}
		}
	}

	// TODO: add fee
	fn transfer(
		transactor: &T::AccountId,
		dest: &T::AccountId,
		value: Self::Balance,
		existence_requirement: ExistenceRequirement,
	) -> Result {
		let new_from_balance = match Self::free_balance(transactor).checked_sub(&value) {
			None => return Err("balance too low to send value"),
			Some(b) => b,
		};

		Self::ensure_can_withdraw(transactor, value, WithdrawReason::Transfer.into(), new_from_balance)?;

		// NOTE: total stake being stored in the same type means that this could never overflow
		// but better to be safe than sorry.
		let new_to_balance = match Self::free_balance(dest).checked_add(&value) {
			Some(b) => b,
			None => return Err("destination balance too high to receive value"),
		};

		if transactor != dest {
			if existence_requirement == ExistenceRequirement::KeepAlive {
				if new_from_balance < Self::minimum_balance() {
					return Err("transfer would kill account");
				}
			}

			Self::set_free_balance(transactor, new_from_balance);
			Self::set_free_balance(dest, new_to_balance);
		}

		Self::deposit_event(RawEvent::TokenTransfer(transactor.clone(), dest.clone(), value));
		Ok(())
	}

	fn slash(who: &T::AccountId, value: Self::Balance) -> (Self::NegativeImbalance, Self::Balance) {
		let free_balance = Self::free_balance(who);
		let free_slash = cmp::min(free_balance, value);

		let new_balance = free_balance - free_slash;

		Self::set_free_balance(who, new_balance);
		let remaining_slash = value - free_slash;

		if !remaining_slash.is_zero() {
			let reserved_balance = Self::reserved_balance(who);
			let reserved_slash = cmp::min(reserved_balance, remaining_slash);
			Self::set_reserved_balance(who, reserved_balance - reserved_slash);
			(
				NegativeImbalance::new(free_slash + reserved_slash),
				remaining_slash - reserved_slash,
			)
		} else {
			(NegativeImbalance::new(value), Zero::zero())
		}
	}

	fn deposit_into_existing(
		who: &T::AccountId,
		value: Self::Balance,
	) -> result::Result<Self::PositiveImbalance, &'static str> {
		if Self::total_balance(who).is_zero() {
			return Err("beneficiary account must pre-exist");
		}
		//add here
		let old_balance = Self::free_balance(who);
		let new_balance = old_balance + value;

		Self::set_free_balance(who, new_balance);
		Ok(PositiveImbalance::new(value))
	}

	fn deposit_creating(who: &T::AccountId, value: Self::Balance) -> Self::PositiveImbalance {
		let old_balance = Self::free_balance(who);
		let new_balance = old_balance + value;

		let (imbalance, _) = Self::make_free_balance_be(who, new_balance);

		if let SignedImbalance::Positive(p) = imbalance {
			p
		} else {
			// Impossible, but be defensive.
			Self::PositiveImbalance::zero()
		}
	}

	fn withdraw(
		who: &T::AccountId,
		value: Self::Balance,
		reasons: WithdrawReasons,
		liveness: ExistenceRequirement,
	) -> result::Result<Self::NegativeImbalance, &'static str> {
		let old_balance = Self::free_balance(who);
		if let Some(new_balance) = old_balance.checked_sub(&value) {
			if liveness == ExistenceRequirement::KeepAlive && new_balance < Self::minimum_balance() {
				return Err("payment would kill account");
			}

			Self::ensure_can_withdraw(who, value, reasons, new_balance)?;
			Self::set_free_balance(who, new_balance);
			Ok(NegativeImbalance::new(value))
		} else {
			Err("too few free funds in account")
		}
	}

	fn make_free_balance_be(
		who: &T::AccountId,
		balance: Self::Balance,
	) -> (
		SignedImbalance<Self::Balance, Self::PositiveImbalance>,
		UpdateBalanceOutcome,
	) {
		let original = Self::free_balance(who);

		let imbalance = if original <= balance {
			SignedImbalance::Positive(PositiveImbalance::new(balance - original))
		} else {
			SignedImbalance::Negative(NegativeImbalance::new(original - balance))
		};

		let outcome = {
			Self::set_free_balance(who, balance);
			UpdateBalanceOutcome::Updated
		};

		(imbalance, outcome)
	}
}

impl<T: Trait> LockableCurrency<T::AccountId> for Module<T>
where
	T::Balance: MaybeSerializeDeserialize + Debug,
{
	type Lock = Lock<T::Balance, Self::Moment>;
	type Moment = T::Moment;
	type WithdrawReasons = WithdrawReasons;

	fn update_lock(who: &T::AccountId, lock: Option<Self::Lock>) -> Self::Balance {
		let at = <timestamp::Module<T>>::now();
		let mut locks = Self::locks(who);
		let expired_locks_amount = if let Some(lock) = lock {
			locks.update_lock(lock, at)
		} else {
			locks.remove_expired_lock(at)
		};
		<Locks<T>>::insert(who, locks);

		expired_locks_amount
	}

	fn remove_locks(who: &T::AccountId, lock: Self::Lock) -> Self::Balance {
		let at = <timestamp::Module<T>>::now();
		locks.remove_locks(at, lock)
	}

	fn can_withdraw(who: &T::AccountId, reasons: Self::WithdrawReasons, new_balance: Self::Balance) -> bool {
		let composite_lock = Self::locks(who);

		if composite_lock.is_empty() {
			return true;
		}

		if {
			let now = <timestamp::Module<T>>::now();
			let mut locked_amount = composite_lock.staking_amount;
			for lock in composite_lock.locks.into_iter() {
				if lock.valid_at(now) && lock.reasons.intersects(reasons) {
					// TODO: check overflow?
					locked_amount += lock.amount;
				}
			}

			new_balance >= locked_amount
		} {
			return true;
		}

		false
	}

	fn locks_count(who: &T::AccountId) -> u32 {
		<Locks<T>>::get(who).locks.len() as _
	}
}
