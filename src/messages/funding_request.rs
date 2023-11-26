use super::ConversionError;
use crate::{
    channel::{fixed_size_payment, PartIdx},
    abiencode::types::{Address, Signature},
    perunwire,
};

const ASSETS: usize = 1;
const PARTICIPANTS: usize = 2;
type State = fixed_size_payment::State<ASSETS, PARTICIPANTS>;
type Params = fixed_size_payment::Params<PARTICIPANTS>;
type Balances = fixed_size_payment::Balances<ASSETS, PARTICIPANTS>;

#[derive(Debug, Clone, Copy)]
pub struct LedgerChannelFundingRequest {
    pub part_idx: PartIdx,
    pub funding_agreement: Balances,
    pub params: Params,
    pub state: State,
}

#[derive(Debug, Clone, Copy)]
pub struct Transaction {
    pub state: State,
    pub sigs: [Signature; PARTICIPANTS],
}

impl TryFrom<perunwire::FundReq> for LedgerChannelFundingRequest {
    type Error = ConversionError;

    fn try_from(value: perunwire::FundReq) -> Result<Self, Self::Error> {
        Ok(Self {
            part_idx: value.idx as usize,
            funding_agreement: value
                .agreement
                .ok_or(ConversionError::ExptectedSome)?
                .try_into()?,
            params: value
                .params
                .ok_or(ConversionError::ExptectedSome)?
                .try_into()?,
            state: value
                .state
                .ok_or(ConversionError::ExptectedSome)?
                .try_into()?,
        })
    }
}

impl From<LedgerChannelFundingRequest> for perunwire::FundReq {
    fn from(value: LedgerChannelFundingRequest) -> Self {
        Self {
            session_id: String::from(""),
            agreement: Some(value.funding_agreement.into()),
            params: Some(value.params.into()),
            state: Some(value.state.into()),
            idx: value.part_idx as u32,
        }
    }
}

impl TryFrom<perunwire::Transaction> for Transaction {
    type Error = ConversionError;

    fn try_from(value: perunwire::Transaction) -> Result<Self, Self::Error> {
        let signed_state = value.state.ok_or(ConversionError::ExptectedSome)?;

        if value.sigs.len() != PARTICIPANTS {
            return Err(ConversionError::ParticipantSizeMissmatch);
        }
        let mut sigs = [Signature::default(); PARTICIPANTS];

        for (a, b) in sigs.iter_mut().zip(value.sigs) {
            *a = Signature(b.try_into().or(Err(ConversionError::ByteLengthMissmatch))?);
        }

        Ok(Self {
            state: signed_state
                .try_into()?,
            sigs,
        })
    }
}

impl From<Transaction> for perunwire::Transaction {
    fn from(value: Transaction) -> Self {
        Self {
            state: Some(value.state.into()),
            sigs: value.sigs.map(|sig| sig.0.to_vec()).to_vec(),
        }
    }
}
