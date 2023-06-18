use super::BEScript;

#[derive(Debug)]
pub enum BEAddress {
    Bitcoin(bitcoin::Address),
    Elements(elements::Address),
}

impl BEAddress {
    pub fn script_pubkey(&self) -> BEScript {
        match self {
            BEAddress::Bitcoin(addr) => addr.script_pubkey().into(),
            BEAddress::Elements(addr) => addr.script_pubkey().into(),
        }
    }
    pub fn blinding_pubkey(&self) -> Option<bitcoin::secp256k1::PublicKey> {
        match self {
            BEAddress::Bitcoin(_) => None,
            BEAddress::Elements(addr) => addr.blinding_pubkey,
        }
    }
    pub fn elements(&self) -> Option<&elements::Address> {
        match self {
            BEAddress::Bitcoin(_) => None,
            BEAddress::Elements(addr) => Some(addr),
        }
    }
    pub fn bitcoin(&self) -> Option<&bitcoin::Address> {
        match self {
            BEAddress::Bitcoin(addr) => Some(addr),
            BEAddress::Elements(_) => None,
        }
    }
}

impl ToString for BEAddress {
    fn to_string(&self) -> String {
        match self {
            BEAddress::Bitcoin(addr) => addr.to_string(),
            BEAddress::Elements(addr) => addr.to_string(),
        }
    }
}
