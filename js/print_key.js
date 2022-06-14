/*const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')
const fs = require('fs').promises
const base32 = require("base32.js")*/
const bs58 = require('bs58')

const anchor = require('@project-serum/anchor')
const provider = anchor.Provider.env()
//const provider = anchor.Provider.local()
anchor.setProvider(provider)
//console.log(tokenAgent)


async function main() {
    let k = provider.wallet.payer.secretKey
    console.log(k)
    console.log(bs58.encode(k))
}

main().then(() => process.exit(0)).catch(error => {
    console.log(error)
})
