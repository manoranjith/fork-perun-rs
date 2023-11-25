use super::ConversionError;
use crate::{
    channel::{fixed_size_payment, PartIdx},
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
