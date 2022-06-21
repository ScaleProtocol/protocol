use anchor_lang::prelude::*;
use bigdecimal::{BigDecimal, ToPrimitive, One};
use byteorder::ByteOrder;

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
    #[msg("Instruction At Wrong Index")]
    InstructionAtWrongIndex,
    #[msg("Invalid Account Data")]
    InvalidAccountData,
    #[msg("Invalid Ed25519 Instruction")]
    InvalidEd25519Instruction,
    #[msg("Invalid Authority")]
    InvalidAuthority,
}

pub const MAX_LEVERAGE: u64 = 100;

#[program]
pub mod protocol {
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
                unimplemented!()
            }
            (Short, Long) => {
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
    ) -> Result<u64> {
        let authenticated = verify_and_extract(&ctx.accounts.instruction_sysvar_account_info)
            .map_err(|_| ProtocolError::InvalidSignature)?;

        let position = &mut ctx.accounts.position;
        position.status = PositionStatus::Processed;

        require_eq!(authenticated.authority, position.authority, ProtocolError::InvalidAuthority);

        let returned_margin = if authenticated.data.is_liquidated {
            position.get_liquidated_margin(authenticated.data.time)
        } else {
            let current_price = get_current_price(
                    &ctx.accounts.price_a, 
                    &ctx.accounts.price_b, 
                    position.decimals)?;
            position.get_profit(&current_price, authenticated.data.time)?
        };

        Ok(returned_margin)
    }
}

#[derive(Debug, Clone, Copy, AnchorDeserialize, AnchorSerialize)]
pub struct PositionArgs {
    pub price: u64,
    pub expo: i32,
    pub decimals: u8,
    pub leverage_margin: u64,
    pub leverage: u64,
    pub ptype: PositionType,
    pub direction: Direction,
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
    pub authority: Pubkey,
    pub index: u32,
    pub status: PositionStatus,
    pub ptype: PositionType,
    pub direction: Direction,
    pub decimals: u8,
    pub leverage: u64,
    pub last_price: i64,
    pub last_conf: u64,
    pub margin: u64,
    pub margin_rate_numerator: u64,
    pub overnight_fee_numerator: u64,
    pub liquidation: u64,
    pub created_at: i64,
    pub slot: u64,
    pub amount: String,
}

impl Position {
    pub const LEN: usize = 32
        + 32
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

#[derive(Debug, Clone, Copy)]
pub struct AuthenticatedData {
    pub data: LiquidatedData,
    pub authority: Pubkey,
}

pub fn verify_and_extract(instruction_sysvar_account_info: &AccountInfo) -> Result<AuthenticatedData> {
    use anchor_lang::solana_program;
    let current_instruction = solana_program::sysvar::instructions::load_current_index_checked(instruction_sysvar_account_info)?;
    if current_instruction == 0 {
        return err!(ProtocolError::InstructionAtWrongIndex);
    }

    let ed25519_ix_index = (current_instruction - 1) as usize;
    let ed25519_ix = solana_program::sysvar::instructions::load_instruction_at_checked(
        ed25519_ix_index,
        instruction_sysvar_account_info,
    )
    .map_err(|_| ProtocolError::InvalidAccountData)?;

    if ed25519_ix.program_id != solana_program::ed25519_program::id() {
        return err!(ProtocolError::InvalidEd25519Instruction);
    }

    let sig_len = ed25519_ix.data[0];
    if sig_len != 1 {
        return err!(ProtocolError::InvalidEd25519Instruction);
    }

    let mut index = 2;
    let _sig_offset = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]) as usize;
    index += 2;
    let sig_ix = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]);
    index += 2;
    let pubkey_offset = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]) as usize;
    index += 2;
    let pubkey_ix = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]);
    index += 2;
    let data_offset = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]) as usize;
    index += 2;
    let data_size = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]) as usize;
    index += 2;
    let data_ix = byteorder::LE::read_u16(&ed25519_ix.data[index..index+2]);

    if pubkey_ix != u16::MAX || data_ix != u16::MAX || sig_ix != u16::MAX {
        return err!(ProtocolError::InvalidEd25519Instruction);
    }

    let authority = Pubkey::try_from_slice(&ed25519_ix.data[pubkey_offset..pubkey_offset + 32])?;
    let data: LiquidatedData = AnchorDeserialize::try_from_slice(&ed25519_ix.data[data_offset..data_offset+data_size])?;

    Ok(AuthenticatedData {
        data,
        authority,
    })
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
        constraint = instruction_sysvar_account_info.key() == anchor_lang::solana_program::sysvar::instructions::id(),
    )]
    pub instruction_sysvar_account_info: AccountInfo<'info>,
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