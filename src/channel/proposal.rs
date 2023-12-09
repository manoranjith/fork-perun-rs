//! Low-level API for the Proposal phase.
//!
//! Currently, this can only handle channels with one asset and two
//! participants. In the future we'll likely generalize it to work with
//! arbitrary channel sizes and potentially even arbitrary ways to represent the
//! data in rust (e.g. using `Vec<T>` vs no-heap `Vec<T>` vs `fixed-size<A,P>`).

use super::{
    agreed_upon::AgreedUponChannel,
    fixed_size_payment::{self},
    NonceShare, PartIdx,
};
use crate::{
    abiencode::{
        self,
        types::{Address, U256},
    },
    messages::{LedgerChannelProposal, LedgerChannelProposalAcc, ParticipantMessage},
    wire::{BroadcastMessageBus, MessageBus},
    PerunClient,
};
use alloc::string::ToString;
use sha3::{Digest, Sha3_256};

const ASSETS: usize = 1;
const PARTICIPANTS: usize = 2;
type State = fixed_size_payment::State<ASSETS, PARTICIPANTS>;
type Params = fixed_size_payment::Params<PARTICIPANTS>;

/// Error returned when the proposal was already accepted by a participant.
#[derive(Debug)]
pub struct AlreadyAcceptedError;

#[derive(Debug)]
pub enum HandleAcceptError {
    InvalidProposalID,
    AlreadyAccepted,
}

/// Error returned when the transition from ProposedChannel -> AgreedUponChannel failed.
#[derive(Debug)]
pub enum ProposalBuildError {
    AbiEncodeError(abiencode::Error),
    MissingAccResponse(PartIdx),
}
impl From<abiencode::Error> for ProposalBuildError {
    fn from(e: abiencode::Error) -> Self {
        Self::AbiEncodeError(e)
    }
}

/// Represents a channel that was proposed, but not accepted by all
/// participants.
///
/// Use `build()` or `try_into()` to get an [AgreedUponChannel], to sign the
/// initial state and exchange those signatures.
#[derive(Debug)]
pub struct ProposedChannel<'cl, B: MessageBus> {
    /// Who are we in this channel (0 is the channel proposer).
    part_idx: PartIdx,
    /// Who should receive funds when withdrawing
    withdraw_receiver: Address,
    /// Reference to the PerunClient, used for communication.
    client: &'cl PerunClient<B>,
    /// Needed for creating the initial state, Params and for the application to
    /// decide if those are valid Parameters.
    proposal: LedgerChannelProposal,
    /// Holds all accept messages received so far.
    ///
    /// The data of Participant 0 is already stored in the proposal. We store
    /// this as an array regardless, to make future transitions to >2 Party
    /// channels easier.
    responses: [Option<LedgerChannelProposalAcc>; 1],
}

impl<'cl, B: MessageBus> ProposedChannel<'cl, B> {
    /// Create a new ProposedChannel.
    ///
    /// The caller ([PerunClient]) is responsible for sending the proposal
    /// message to all participants.
    pub(crate) fn new(
        client: &'cl PerunClient<B>,
        part_idx: PartIdx,
        withdraw_receiver: Address,
        proposal: LedgerChannelProposal,
    ) -> Self {
        ProposedChannel {
            part_idx,
            withdraw_receiver,
            client,
            proposal,
            responses: [None],
        }
    }

    /// Accept a proposed channel and reply to the participants.
    ///
    /// Do not call this if you have proposed the channel yourself, it will just
    /// return an Error.
    pub fn accept(
        &mut self,
        nonce_share: NonceShare,
        address: Address,
    ) -> Result<(), AlreadyAcceptedError> {
        // In go-perun this "can we sign it" is checked in `completeCPP` by
        // trying to unlock the corresponding wallet.
        // assert_eq!(address, self.client.signer.address(), "We have to be able to sign things with this address and the current implementation is only able to have a single singer address. It is still part of the accept function signature because this will probably change in the future and this change would be backwards incompatible.");

        // if self.part_idx == 0 || self.responses[self.part_idx - 1].is_some() {
            // return Err(AlreadyAcceptedError);
        // }

        let acc: _ = LedgerChannelProposalAcc {
            proposal_id: self.proposal.proposal_id,
            nonce_share,
            participant: address,
        };
        self.responses[self.part_idx - 1] = Some(acc);
        self.client.bus.broadcast_to_participants(
            self.part_idx,
            &self.proposal.peers,
            ParticipantMessage::ProposalAccepted(acc),
        );

        Ok(())
    }

    /// Reject a proposed channel and send the reply to the participants.
    ///
    /// Drops the ProposedChannel object because using it no longer makes sense,
    /// as we have rejected the proposal.
    pub fn reject(self, reason: &str) {
        self.client.bus.broadcast_to_participants(
            self.part_idx,
            &self.proposal.peers,
            ParticipantMessage::ProposalRejected {
                id: self.proposal.proposal_id,
                reason: reason.to_string(),
            },
        );
    }

    /// Call this when receiving an Accept response form a participant.
    ///
    /// Adds the response to the list of responses, needed to progress to the
    /// next Phase: Creating and signing the initial state.
    ///
    /// When receiving a reject message, the [ProposedChannel] object can be
    /// dropped.
    pub fn participant_accepted(
        &mut self,
        part_idx: PartIdx,
        msg: LedgerChannelProposalAcc,
    ) -> Result<(), HandleAcceptError> {
        if msg.proposal_id != self.proposal.proposal_id {
            return Err(HandleAcceptError::InvalidProposalID);
        }

        let index = part_idx - 1;
        match self.responses[index] {
            Some(_) => Err(HandleAcceptError::AlreadyAccepted),
            None => {
                self.responses[index] = Some(msg);
                Ok(())
            }
        }
    }

    /// Progress to the next phase: Signing the initial state.
    ///
    /// This does **not** enforce channel_id uniqueness. Though exactly the same
    /// channel_id is unlikely due to using different nonces. It is up to the
    /// caller to handle this if he handles multiple channels and uses the
    /// channel_id for forwarding messages to the correct channel (and having
    /// multiple channels with the same channel_id will be problematic
    /// on-chain). Checking this is not the task of this class, which is only
    /// concerned about a single channel. Go-perun does this check in
    /// `completeCPP`.
    ///
    /// In the case of an error we still want the caller to be able to recover
    /// from it, so we have to give self back. If we wouldn't do that the caller
    /// would be forced to (implicitly) throw away the entire channel, so we
    /// could just as well have paniced in case of an error.
    pub fn build(self) -> Result<AgreedUponChannel<'cl, B>, (Self, ProposalBuildError)> {
        let mut participants = [Address::default(); PARTICIPANTS];
        participants[0] = self.proposal.participant;

        // Go-Perun does NOT use keccak256 here, probably to be less dependent
        // on Ethereum. We have to do the same here.
        let mut hasher = Sha3_256::new();
        hasher.update(self.proposal.nonce_share.0);

        // Go through all responses and make sure none is missing. Additionally
        // collect information needed later.
        //
        // Call combining it into a single loop premature optimization if you
        // want, but I didn't like that two loops either required to call
        // `unwrap()` or `unwrap_unchecked()`, while not knowing if the compiler
        // combines it into one loop. Nor did I find a good way to do this
        // purely with iterators. This might even be more readable than two
        // loops using `unwrap()` or `unwrap_unchecked()`, where you have to
        // argue why it is save to do so. (I didn't want to introduce another
        // intermediate representation array, which I don't know if the compiler
        // would optimize away).
        for (index, res) in self.responses.iter().enumerate() {
            // Unwrap all responses, returning an error if one is missing
            let res = match res {
                Some(v) => v,
                None => return Err((self, ProposalBuildError::MissingAccResponse(index + 1))),
            };

            // Store in new participants list that doesn't use options and
            // combine the nonces
            participants[index + 1] = res.participant;
            hasher.update(res.nonce_share.0);
        }

        // Finalize the nonce.
        let nonce = U256::from_big_endian(hasher.finalize().as_slice());

        // Create the initial state
        let params: Params = Params {
            challenge_duration: self.proposal.challenge_duration,
            nonce,
            participants,
            app: Address([0u8; 20]),
            ledger_channel: true,
            virtual_channel: false,
        };
        let init_state = match State::new(params, self.proposal.init_bals) {
            Ok(v) => v,
            Err(e) => return Err((self, e.into())),
        };

        Ok(AgreedUponChannel::new(
            self.client,
            self.proposal.funding_agreement,
            self.part_idx,
            self.withdraw_receiver,
            init_state,
            params,
            self.proposal.peers,
        ))
    }
}

impl<'cl, B: MessageBus> TryFrom<ProposedChannel<'cl, B>> for AgreedUponChannel<'cl, B> {
    type Error = (ProposedChannel<'cl, B>, ProposalBuildError);

    fn try_from(value: ProposedChannel<'cl, B>) -> Result<Self, Self::Error> {
        value.build()
    }
}
