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
    const allowanceData = new PublicKey('25NNEHFiEpRL7JosvruvXYH8Supgd8iuKtfhyEdP62YQ')

    /*var act = await tokenAgent.account.tokenAllowance.fetch(allowanceData)
    console.log('Initial Allowance Data')
    console.log(act)*/

    console.log('Close Allowance')
    let txsig = await tokenAgent.rpc.closeAllowance(
        {
            accounts: {
                allowanceData: allowanceData,
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
