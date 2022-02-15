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

async function main() {
    const subscrData = new PublicKey('9SmeJPYfufJyGbJQvvLmkEPnDmF5QAA5JqQ79Ltcnf92')

    var act = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log('Initial Subscription Data')
    console.log(act)

    console.log('Close Subscription')
    let txsig = await tokenAgent.rpc.closeSubscription(
        {
            accounts: {
                //subscrData: new PublicKey('Fxg4sFxmiWFPaxS7Xtgnk4J83grzcky9ZpMd6GyutEPd'),
                subscrData: subscrData,
                userKey: provider.wallet.publicKey,
                feeRecipient: provider.wallet.publicKey,
            },
        }
    )
    console.log(txsig)
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
