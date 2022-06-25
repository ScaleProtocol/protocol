use crate::Direction;
use crate::PositionType;
use crate::Rate;

use std::convert::From;

pub struct ProtocolPrice {
    pub price: i64,
    pub conf: u64,
    pub expo: i32,
}

impl From<pyth_sdk_solana::Price> for ProtocolPrice {
    fn from(pyth_price: pyth_sdk_solana::Price) -> Self {
        Self {
            price: pyth_price.price,
            conf: pyth_price.conf,
            expo: pyth_price.expo,
        }
    }
}

pub struct TransactionAccount {
    pub direction: Direction,
    pub ptype: PositionType,
    pub initial_shares_price: ProtocolPrice,
    pub asset_decimals: u32,
    pub shares_with_decimals: u64,
    pub leverage: u64,
    pub financing_rate: Rate,
}

impl TransactionAccount {

    pub fn initial_price(&self) -> i64 {
        self.initial_shares_price.price
    }

    pub fn inital_conf(&self) -> u64 {
        self.initial_shares_price.conf
    }

    fn shares(&self) -> u64 {
        self.shares_with_decimals
    }

    pub fn initial_margin(&self) -> Option<u64> {
        self.shares()
            .checked_mul(self.initial_price() as u64)?
            .checked_div(self.leverage)?
            .checked_div(10u64.pow(self.asset_decimals as u32))

    }

    pub fn buy_to_open_price(&self) -> Option<i64> {
        if self.direction == Direction::OpenLong {
            return self
                .initial_price()
                .checked_add(
                    self.inital_conf()
                        .try_into()
                        .ok()?
                );
        }
        None
    }

    pub fn sell_to_open_price(&self) -> Option<i64> {
        if self.direction == Direction::OpenShort {
            return self
                .initial_price()
                .checked_sub(
                    self.inital_conf()
                        .try_into()
                        .ok()?
                );
        }
        None
    }

    /*
    .checked_sub(
        self.initial_margin()?
            .checked_mul(self.leverage)?
            .checked_mul(self.financing_rate.numerator)?
            .checked_div(self.financing_rate.denominator)?
            .checked_mul(days)?
            .checked_div(365)?
            .try_into()
            .ok()?
    )
    */
    pub fn sell_to_close_profit(&self, price: &pyth_sdk_solana::Price) -> Option<i128> {
        if self.direction == Direction::OpenLong {
            let diff = (price.price)
                .checked_sub(price.conf as i64)?
                .checked_sub(self.buy_to_open_price()?)?;
            dbg!(diff);
            return (self.shares() as i128)
                .checked_mul(diff as i128)?
                .checked_div(10i128.pow(self.asset_decimals));
                
        }
        None
    }

    /*
    .checked_sub(
        self.initial_margin()?
            .checked_mul(self.leverage)?
            .checked_mul(self.financing_rate.numerator)?
            .checked_div(self.financing_rate.denominator)?
            .checked_mul(days)?
            .checked_div(365)?
            .try_into()
            .ok()?
    )
     */
    pub fn buy_to_close_profit(&self, price: &pyth_sdk_solana::Price) -> Option<i128> {
        if self.direction == Direction::OpenShort {
            let diff = self.sell_to_open_price()?
                .checked_sub(
                    price.price
                        .checked_add(price.conf as i64)?
                )?;
            return (self.shares() as i128)
                .checked_mul(diff as i128)?
                .checked_div(10i128.pow(self.asset_decimals));
        }
        None
    }

    pub fn get_profit(&self, price: &pyth_sdk_solana::Price, days: u64) -> Option<i128> {
        let gross = match self.direction {
            Direction::OpenLong => self.sell_to_close_profit(price)?,
            Direction::OpenShort => self.buy_to_close_profit(price)?,
        };

        gross
            .checked_sub(
                self.initial_margin()?
                    .checked_mul(self.leverage)?
                    .checked_mul(self.financing_rate.numerator)?
                    .checked_div(self.financing_rate.denominator)?
                    .checked_mul(days)?
                    .checked_div(365)?
                    .try_into()
                    .ok()?
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_price() {
        let btc = pyth_sdk_solana::Price {
            price: 30000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day1 = btc.get_price_in_quote(&usdc, -6).unwrap();

        let btc2 = pyth_sdk_solana::Price {
            price: 32000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc2 = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day10 = btc2.get_price_in_quote(&usdc2, -6).unwrap();

        assert!(price_day1.price == 30000_000_000);
        assert!(price_day1.expo == -6);
        assert!(price_day10.price == 32000_000_000);
        assert!(price_day10.expo == -6);
    }

    #[test]
    fn test_open_long() {
        let btc = pyth_sdk_solana::Price {
            price: 30000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day1 = btc.get_price_in_quote(&usdc, -6).unwrap();

        let account = TransactionAccount {
            direction: Direction::OpenLong,
            ptype: PositionType::Isolated,
            initial_shares_price: price_day1.into(),
            asset_decimals: 6,
            shares_with_decimals: 1000000,
            leverage: 100,
            financing_rate: Rate { numerator: 300, denominator: 10000 },
        };

        dbg!(account.initial_margin());
        dbg!(account.initial_price());
        dbg!(account.buy_to_open_price());
        assert!(account.sell_to_open_price().is_none());

        let btc2 = pyth_sdk_solana::Price {
            price: 32000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc2 = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day10 = btc2.get_price_in_quote(&usdc2, -6).unwrap();

        dbg!(account.sell_to_close_profit(&price_day10));
        assert!(account.buy_to_close_profit(&price_day10).is_none());

        let btc3 = pyth_sdk_solana::Price {
            price: 29800_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc3 = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day5 = btc3.get_price_in_quote(&usdc3, -6).unwrap();

        dbg!(account.get_profit(&price_day5, 5));
    }

    #[test]
    fn test_open_short() {
        let btc = pyth_sdk_solana::Price {
            price: 30000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day1 = btc.get_price_in_quote(&usdc, -6).unwrap();

        let account = TransactionAccount {
            direction: Direction::OpenShort,
            ptype: PositionType::Isolated,
            initial_shares_price: price_day1.into(),
            asset_decimals: 6,
            shares_with_decimals: 1000000,
            leverage: 100,
            financing_rate: Rate { numerator: 300, denominator: 10000 },
        };

        dbg!(account.initial_margin());
        dbg!(account.initial_price());
        assert!(account.buy_to_open_price().is_none());
        dbg!(account.sell_to_open_price());

        let btc2 = pyth_sdk_solana::Price {
            price: 28000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc2 = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day5 = btc2.get_price_in_quote(&usdc2, -6).unwrap();

        assert!(account.sell_to_close_profit(&price_day5).is_none());
        dbg!(account.get_profit(&price_day5, 5));

        let btc3 = pyth_sdk_solana::Price {
            price: 30200_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc3 = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day10 = btc3.get_price_in_quote(&usdc3, -6).unwrap();

        dbg!(account.get_profit(&price_day10, 10));
        assert!(account.get_profit(&price_day10, 10).unwrap() < -200_000_000);
    }

    #[test]
    fn test_open_long_and_sell_quickly() {
        let btc = pyth_sdk_solana::Price {
            price: 30000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day1 = btc.get_price_in_quote(&usdc, -6).unwrap();

        dbg!(price_day1);

        let account = TransactionAccount {
            direction: Direction::OpenLong,
            ptype: PositionType::Isolated,
            initial_shares_price: price_day1.into(),
            asset_decimals: 6,
            shares_with_decimals: 1000000,
            leverage: 100,
            financing_rate: Rate { numerator: 300, denominator: 10000 },
        };

        dbg!(account.buy_to_open_price());
        dbg!(account.get_profit(&price_day1, 1));
    }

    #[test]
    fn test_open_short_and_buy_quickly() {
        let btc = pyth_sdk_solana::Price {
            price: 30000_0000_0000,
            conf: 5_0000_0000,
            expo: -8,
        };

        let usdc = pyth_sdk_solana::Price {
            price: 1_0000_0000,
            conf: 2_5000,
            expo: -8,
        };

        let price_day1 = btc.get_price_in_quote(&usdc, -6).unwrap();

        dbg!(price_day1);

        let account = TransactionAccount {
            direction: Direction::OpenShort,
            ptype: PositionType::Isolated,
            initial_shares_price: price_day1.into(),
            asset_decimals: 6,
            shares_with_decimals: 1000000,
            leverage: 100,
            financing_rate: Rate { numerator: 300, denominator: 10000 },
        };

        let profit = account.get_profit(&price_day1, 1);
        dbg!(account.sell_to_open_price());
        dbg!(profit);

        assert!(profit.unwrap().is_negative());
    }
}