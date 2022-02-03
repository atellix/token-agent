const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')
const fs = require('fs').promises
const base32 = require("base32.js")

const anchor = require('@project-serum/anchor')
const provider = anchor.Provider.env()
//const provider = anchor.Provider.local()
anchor.setProvider(provider)
const tokenAgent = anchor.workspace.TokenAgent
const tokenAgentPK = tokenAgent.programId
//console.log(tokenAgent)

async function programAddress(inputs, program) {
    const addr = await PublicKey.findProgramAddress(inputs, program)
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

async function main() {
    var merchantPK = new PublicKey('2cxhShPFPqqPyZngmLEoujX173J6JT1gSHd9wvpATV5r')
    var drop = await programAddress([merchantPK.toBuffer()], tokenAgentPK)
    console.log('Merchant Drop: ' + drop.pubkey)
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
