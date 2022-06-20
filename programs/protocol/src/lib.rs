use anchor_lang::prelude::*;
use bytemuck::{bytes_of, Pod, Zeroable};
use solana_sdk::instruction::Instruction;
use bigdecimal::{BigDecimal, ToPrimitive, One};

declare_id!("EuKUep9dcVnTbXHoX3UxpBbrJXY3nVAz1THwwHjtuMp1");

#[error_code]
pub enum ProtocolError {
    #[msg("Invalid Price")]
    InvalidPrice,
    #[msg("Invalid Price Account")]
    InvalidPriceAccount,
    #[msg("Slippage Reached")]
    SlippageReached,
    #[msg("Insufficient Balance")]
    InsufficientBalance,
    #[msg("Invalid Leverage")]
    InvalidLeverage,
    #[msg("Position Liquidated")]
    PositionLiquidated,
    #[msg("Invalid Args")]
    InvalidArgs,
    #[msg("Invalid Signature")]
    InvalidSignature,
}

pub const MAX_LEVERAGE: u64 = 100;

#[program]
pub mod scale_protocol {
    use super::*;

    pub fn create(ctx: Context<Create>, index: u32, args: PositionArgs) -> Result<()> {
        let position = &mut ctx.accounts.position;
        position.status = PositionStatus::Open;
        position.pool = ctx.accounts.pool.key();
        position.owner = ctx.accounts.payer.key();
        position.index = index;
        position.margin = args.margin();
        position.ptype = args.ptype;
        position.direction = args.direction;
        position.created_at = Clock::get()?.unix_timestamp;
        position.slot = Clock::get()?.slot;
        position.decimals = args.decimals;

        if args.leverage > MAX_LEVERAGE {
            return err!(ProtocolError::InvalidLeverage);
        }

        let current_price = get_current_price(&ctx.accounts.price_a, &ctx.accounts.price_b, args.decimals)?;
        position.last_price = current_price.price;
        position.liquidation = get_liquidation(
            current_price.price,
            position.bond(),
            args.direction,
        );

        match args.ptype {
            PositionType::Isolated => {
                match args.direction {
                    Direction::Long => {
                        let ask = BigDecimal::from(
                            (current_price.price as u64)
                                .checked_add(current_price.conf)
                                .ok_or(ProtocolError::InvalidPrice)?
                        );

                        check_slippage(&ask, args)?;

                        position.amount = get_asset_amount(args.leverage_margin, &ask).to_string();
                    }
                    Direction::Short => {
                        let bid = BigDecimal::from(
                            (current_price.price as u64)
                                .checked_sub(current_price.conf)
                                .ok_or(ProtocolError::InvalidPrice)?
                        );

                        check_slippage(&bid, args)?;

                        position.amount = get_asset_amount(args.leverage_margin, &bid).to_string();
                    }
                }
            }
            PositionType::Cross => unimplemented!(),
        };

        Ok(())
    }

    /// TODO
    pub fn netoff(ctx: Context<Netoff>, args: PositionArgs) -> Result<()> {
        let position = &mut ctx.accounts.position;

        if args.leverage > MAX_LEVERAGE {
            return err!(ProtocolError::InvalidLeverage);
        }

        let current_price = get_current_price(&ctx.accounts.price_a, &ctx.accounts.price_b, args.decimals)?;
        if position.is_liquidated(current_price.price as u64) {
            return err!(ProtocolError::PositionLiquidated);
        }

        use Direction::*;
        match (args.direction, position.direction) {
            (Long, Long) => {
                unimplemented!()
            }
            (Short, Short) => {
                unimplemented!()
            }
            (Long, Short) => {
                // can only be sold on lower price
                unimplemented!()
            }
            (Short, Long) => {
                // can only be bought on higher price
                unimplemented!()
            }
        }
    }

    pub fn increase_margin(ctx: Context<IncreaseMargin>, amount: u64) -> Result<()> {
        let position = &mut ctx.accounts.position;

        let current_price = get_current_price(&ctx.accounts.price_a, &ctx.accounts.price_b, position.decimals)?;
        if position.is_liquidated(current_price.price as u64) {
            return err!(ProtocolError::PositionLiquidated);
        }
        position.margin += amount;

        position.liquidation = get_liquidation(
            position.last_price,
            position.bond(),
            position.direction,
        );

        Ok(())
    }

    pub fn process_position<'info>(
        ctx: Context<'_, '_, '_, 'info, ProcessPosition<'info>>, 
        data: ProcessData,
    ) -> Result<u64> {
        data.verify()
            .map_err(|_| ProtocolError::InvalidSignature)?;
        // assume that the position will be successfully processed
        // set the position status into processed, give the close choice to user if they want delete the record
        let position = &mut ctx.accounts.position;
        position.status = PositionStatus::Processed;

        // check args if the position is liquidated and skip the profit calculation
        let returned_margin = if data.message.is_liquidated {
            position.get_liquidated_margin(data.message.time)
        } else {
            let current_price = get_current_price(
                    &ctx.accounts.price_a, 
                    &ctx.accounts.price_b, 
                    position.decimals)?;
            position.get_profit(&current_price, data.message.time)?
        };

        Ok(returned_margin)
    }
}

#[derive(Debug, Clone, Copy, AnchorDeserialize, AnchorSerialize)]
pub struct PositionArgs {
    // the ask/bid price when user signed the transaction
    pub price: u64,
    pub expo: i32,
    pub decimals: u8,
    // the amount of Y user want to buy/sell, include the leverage, so the user paid is hands / leverage
    pub leverage_margin: u64,
    // max 100
    pub leverage: u64,
    // isolated or cross
    pub ptype: PositionType,
    // long or short
    pub direction: Direction,
    // the real ask/bid price user can accept
    pub slippage_numerator: u64,
    pub margin_rate_numerator: u64,
}
impl PositionArgs {
    pub fn margin(&self) -> u64 {
        self.leverage_margin
            .checked_div(self.leverage)
            .unwrap()
    }
}

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, AnchorDeserialize, AnchorSerialize)]
pub enum PositionType {
    // isolated-margin
    Isolated,
    // cross-margin
    Cross,
}
#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, AnchorDeserialize, AnchorSerialize)]
pub enum Direction {
    Long,
    Short,
}

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, AnchorDeserialize, AnchorSerialize)]
pub enum PositionStatus {
    Open,
    Processed,
}

#[account]
#[derive(Debug)]
pub struct Position {
    pub pool: Pubkey,
    pub owner: Pubkey,
    pub index: u32,
    pub status: PositionStatus,
    pub ptype: PositionType,
    pub direction: Direction,
    pub decimals: u8,
    pub leverage: u64,
    pub last_price: i64,
    pub margin: u64,
    pub margin_rate_numerator: u64,
    pub overnight_fee_numerator: u64,
    pub liquidation: u64,
    pub created_at: i64,
    pub slot: u64,
    // the amount of asset user hold on, include the leverage
    pub amount: String,
}

impl Position {
    pub const LEN: usize = 32
        + 32
        + 4
        + 1 + 1 + 1 + 1
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 8
        + 200;

    #[inline(always)]
    pub fn amount(&self) -> BigDecimal {
        std::str::FromStr::from_str(&self.amount).unwrap()
    }

    #[inline(always)]
    pub fn is_liquidated(&self, price: u64) -> bool {
        match self.direction {
            Direction::Long => price <= self.liquidation,
            Direction::Short => price >= self.liquidation,
        }
    }

    #[inline(always)]
    pub fn maintainance_margin(&self) -> u64 {
        self.margin
            .checked_mul(self.margin_rate_numerator).unwrap()
            .checked_div(10000).unwrap()
    }

    #[inline(always)]
    pub fn overnight_fee(&self, time: i64) -> u64 {
        let days = time
            .checked_sub(self.created_at).unwrap()
            .checked_add(86400).unwrap()
            .checked_div(86400).unwrap() as u64;
        let assets = self.amount() * BigDecimal::from(self.leverage);
        (assets * BigDecimal::from(days) * BigDecimal::from(self.overnight_fee_numerator) / BigDecimal::from(10000)).to_u64().unwrap()
    }

    #[inline(always)]
    pub fn bond(&self) -> u64 {
        self.margin - self.maintainance_margin()
    }

    pub fn get_liquidated_margin(&self, time: i64) -> u64 {
        let overnight_fee = self.overnight_fee(time);
        self.maintainance_margin()
            .checked_sub(overnight_fee as u64).unwrap()
    }

    pub fn get_profit(&self, current_price: &pyth_sdk_solana::Price, time: i64) -> Result<u64> {
        let sold_price = current_price.price
            .checked_sub(current_price.conf as i64)
            .ok_or(ProtocolError::InvalidPrice)?;
        Ok(match self.direction {
            Direction::Long => {
                if sold_price < self.last_price {
                    // loss
                    let difference = self.last_price - sold_price;
                    self.margin
                        .checked_sub(difference as u64)
                        .ok_or(ProtocolError::InvalidPrice)?
                        .checked_sub(self.overnight_fee(time))
                        .ok_or(ProtocolError::InvalidPrice)?
                } else {
                    // earned
                    let earned = (BigDecimal::from(sold_price) * self.amount())
                        .to_u64()
                        .ok_or(ProtocolError::InvalidPrice)?;
                    self.margin
                        .checked_sub(self.overnight_fee(time))
                        .ok_or(ProtocolError::InvalidPrice)?
                        .checked_add(earned)
                        .ok_or(ProtocolError::InvalidPrice)?
                }
            }
            Direction::Short => {
                let bought_price = current_price.price
                    .checked_add(current_price.conf as i64)
                    .ok_or(ProtocolError::InvalidPrice)?;
                if bought_price > self.last_price {
                    // loss
                    let difference = bought_price - self.last_price;
                    self.margin
                        .checked_sub(difference as u64)
                        .ok_or(ProtocolError::InvalidPrice)?
                        .checked_sub(self.overnight_fee(time))
                        .ok_or(ProtocolError::InvalidPrice)?
                } else {
                    // earned
                    let earned = (BigDecimal::from(sold_price) * self.amount())
                        .to_u64()
                        .ok_or(ProtocolError::InvalidPrice)?;
                    self.margin
                        .checked_sub(self.overnight_fee(time))
                        .ok_or(ProtocolError::InvalidPrice)?
                        .checked_add(earned)
                        .ok_or(ProtocolError::InvalidPrice)?
                }
            }
        })
    }
}

#[derive(Accounts)]
#[instruction(index: u32)]
pub struct Create<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    /// CHECK:
    pub pool: UncheckedAccount<'info>,
    /// CHECK:
    pub price_a: UncheckedAccount<'info>,
    /// CHECK:
    pub price_b: UncheckedAccount<'info>,
    #[account(init,
        seeds = [b"protocol", payer.key().as_ref(), index.to_le_bytes().as_ref()],
        bump,
        payer = payer,
        space = 8 + Position::LEN,
    )]
    pub position: Account<'info, Position>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(args: PositionArgs)]
pub struct Netoff<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub price_a: UncheckedAccount<'info>,
    pub price_b: UncheckedAccount<'info>,
    #[account(mut,
        constraint = position.owner == payer.key(),
        constraint = position.ptype == PositionType::Isolated,
        constraint = args.ptype == PositionType::Isolated,
    )]
    pub position: Account<'info, Position>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct IncreaseMargin<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub price_a: UncheckedAccount<'info>,
    pub price_b: UncheckedAccount<'info>,
    #[account(mut,
        constraint = position.owner == payer.key(),
    )]
    pub position: Account<'info, Position>,
    pub system_program: Program<'info, System>,
}

#[derive(Debug, Clone, Copy, PartialOrd, PartialEq, AnchorDeserialize, AnchorSerialize)]
pub struct LiquidatedData {
    pub is_liquidated: bool,
    pub price: u64,
    pub time: i64,
    pub slot: u64,
}
#[derive(Debug, Clone, PartialOrd, PartialEq, AnchorDeserialize, AnchorSerialize)]
pub struct ProcessData {
    pub message: LiquidatedData,
    pub pubkey: Pubkey,
    pub signature: Vec<u8>,
}

impl ProcessData {
    pub fn verify(&self) -> Result<()> {
        let mut buf = vec![];
        self.message.serialize(&mut buf)?;
        anchor_lang::solana_program::program::invoke(
            &new_ed25519_program_instruction(
                &self.signature,
                &self.pubkey.to_bytes(),
                &buf,
            ),
            &[],
        ).map_err(|e| e.into())
    }
}


#[derive(Accounts)]
pub struct ProcessPosition<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    pub pool: UncheckedAccount<'info>,
    pub price_a: UncheckedAccount<'info>,
    pub price_b: UncheckedAccount<'info>,
    #[account(mut,
        close = payer,
        constraint = position.owner == payer.key(),
    )]
    pub position: Account<'info, Position>,
    pub system_program: Program<'info, System>,
    #[account(
        constraint = ed25519_program.key() == anchor_lang::solana_program::ed25519_program::id(),
    )]
    pub ed25519_program: AccountInfo<'info>,
}

fn get_current_price<'a>(price_a: &'a UncheckedAccount, price_b: &'a UncheckedAccount, decimals: u8) -> Result<pyth_sdk_solana::Price> {
    // price feed
    let pfa = pyth_sdk_solana::load_price_feed_from_account_info(price_a)
        .map_err(|_| ProtocolError::InvalidPriceAccount)?;
    let pfb = pyth_sdk_solana::load_price_feed_from_account_info(price_b)
        .map_err(|_| ProtocolError::InvalidPriceAccount)?;
    // current price
    let cpa = pfa
        .get_current_price()
        .ok_or(ProtocolError::InvalidPrice)?;
    let cpb = pfb
        .get_current_price()
        .ok_or(ProtocolError::InvalidPrice)?;
    let target_expo = (decimals as i32)
        .checked_neg()
        .ok_or(ProtocolError::InvalidArgs)?;
    cpa.get_price_in_quote(&cpb, target_expo)
        .ok_or(ProtocolError::InvalidPrice.into())
}

fn get_liquidation(price: i64, bond: u64, direction: Direction) -> u64 {
    // the price
    match direction {
        Direction::Long => {
            (price as u64)
                .checked_sub(bond).unwrap()
        }
        Direction::Short => {
            (price as u64)
                .checked_add(bond).unwrap()
        }
    }
}

fn get_asset_amount(leverage_margin: u64, price: &BigDecimal) -> BigDecimal {
    BigDecimal::from(leverage_margin) / price
}

fn check_slippage<'a>(price: &'a BigDecimal, args: PositionArgs) -> Result<()> {
    let slippage = BigDecimal::from(args.slippage_numerator) / BigDecimal::from(10000u64);
    let price_before = BigDecimal::from(
        args.price
            .checked_div(
                10u64
                    .checked_pow(args.expo.abs() as u32)
                    .ok_or(ProtocolError::InvalidPrice)?
            )
            .ok_or(ProtocolError::InvalidPrice)?
    );
    let one = BigDecimal::one();

    match args.direction {
        Direction::Long => {
            // the real price is higher than the given price
            if price.cmp(&(price_before * (one + slippage))).is_ge() {
                return err!(ProtocolError::SlippageReached);
            }
        }
        Direction::Short => {
            // the real price is lower than the given price
            if price.cmp(&(price_before * (one - slippage))).is_le() {
                return err!(ProtocolError::SlippageReached);
            }
        }
    }
    Ok(())
}

pub const PUBKEY_SERIALIZED_SIZE: usize = 32;
pub const SIGNATURE_SERIALIZED_SIZE: usize = 64;
pub const SIGNATURE_OFFSETS_SERIALIZED_SIZE: usize = 14;
// bytemuck requires structures to be aligned
pub const SIGNATURE_OFFSETS_START: usize = 2;
pub const DATA_START: usize = SIGNATURE_OFFSETS_SERIALIZED_SIZE + SIGNATURE_OFFSETS_START;
#[derive(Default, Debug, Copy, Clone, Zeroable, Pod)]
#[repr(C)]
pub struct Ed25519SignatureOffsets {
    signature_offset: u16,             // offset to ed25519 signature of 64 bytes
    signature_instruction_index: u16,  // instruction index to find signature
    public_key_offset: u16,            // offset to public key of 32 bytes
    public_key_instruction_index: u16, // instruction index to find public key
    message_data_offset: u16,          // offset to start of message data
    message_data_size: u16,            // size of message data
    message_instruction_index: u16,    // index of instruction data to get message data
}

fn new_ed25519_program_instruction<'a>(signature: &'a [u8], pubkey: &'a [u8], message: &'a [u8]) -> Instruction {
    assert_eq!(pubkey.len(), PUBKEY_SERIALIZED_SIZE);
    assert_eq!(signature.len(), SIGNATURE_SERIALIZED_SIZE);

    let mut instruction_data = Vec::with_capacity(
        DATA_START
            .saturating_add(SIGNATURE_SERIALIZED_SIZE)
            .saturating_add(PUBKEY_SERIALIZED_SIZE)
            .saturating_add(message.len()),
    );

    let num_signatures: u8 = 1;
    let public_key_offset = DATA_START;
    let signature_offset = public_key_offset.saturating_add(PUBKEY_SERIALIZED_SIZE);
    let message_data_offset = signature_offset.saturating_add(SIGNATURE_SERIALIZED_SIZE);

    // add padding byte so that offset structure is aligned
    instruction_data.extend_from_slice(bytes_of(&[num_signatures, 0]));

    let offsets = Ed25519SignatureOffsets {
        signature_offset: signature_offset as u16,
        signature_instruction_index: u16::MAX,
        public_key_offset: public_key_offset as u16,
        public_key_instruction_index: u16::MAX,
        message_data_offset: message_data_offset as u16,
        message_data_size: message.len() as u16,
        message_instruction_index: u16::MAX,
    };

    instruction_data.extend_from_slice(bytes_of(&offsets));

    debug_assert_eq!(instruction_data.len(), public_key_offset);

    instruction_data.extend_from_slice(&pubkey);

    debug_assert_eq!(instruction_data.len(), signature_offset);

    instruction_data.extend_from_slice(&signature);

    debug_assert_eq!(instruction_data.len(), message_data_offset);

    instruction_data.extend_from_slice(message);

    Instruction {
        program_id: anchor_lang::solana_program::ed25519_program::id(),
        accounts: vec![],
        data: instruction_data,
    }
}