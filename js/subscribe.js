const { Buffer } = require('buffer')
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { PublicKey } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')

const anchor = require('@project-serum/anchor')
//const provider = anchor.Provider.env()
const provider = anchor.Provider.local()
anchor.setProvider(provider)
const tokenAgent = anchor.workspace.TokenAgent
const tokenAgentPK = tokenAgent.programId

console.log(tokenAgent)

async function main() {
    const subscrId = uuidv4()
    const subscrData = anchor.web3.Keypair.generate()
    const subscrDataBytes = tokenAgent.account.subscrData.size
    const subscrDataRent = await provider.connection.getMinimumBalanceForRentExemption(subscrDataBytes)
    console.log('Subscr Data Rent: ' + subscrDataRent)
    const merchantPK = anchor.web3.Keypair.generate()
    const tokenMint = new PublicKey('7KCJVP436UCWf4qT4Nc6ora62ZqsYtadyft47QLmFUHL')
    const tokenAccount = new PublicKey('44EkCqNcJJZA7h5aaPDTnmj1cuLgigdxUbXdhpBX58nk')
    const rebillData = anchor.web3.Keypair.generate()
    const rebillDataBytes = 66 + (512 * 2)
    const rebillDataRent = await provider.connection.getMinimumBalanceForRentExemption(rebillDataBytes)
    console.log('Rebill Data Rent: ' + rebillDataRent)
    const tx = new anchor.web3.Transaction()
    tx.add(
        anchor.web3.SystemProgram.createAccount({
            fromPubkey: provider.wallet.publicKey,
            newAccountPubkey: subscrData.publicKey,
            space: subscrDataBytes,
            lamports: subscrDataRent,
            programId: tokenAgentPK,
        })
    )
    await provider.send(tx, [subscrData])
    const tx2 = new anchor.web3.Transaction()
    tx2.add(
        anchor.web3.SystemProgram.createAccount({
            fromPubkey: provider.wallet.publicKey,
            newAccountPubkey: rebillData.publicKey,
            space: rebillDataBytes,
            lamports: rebillDataRent,
            programId: tokenAgentPK,
        })
    )
    await provider.send(tx2, [rebillData])
    console.log('Subscribe')
    await tokenAgent.rpc.createSubscription(
        new anchor.BN(uuidparse(subscrId)), // inp_subscr_uuid
        2,                                  // inp_period
        new anchor.BN(10000),               // inp_budget
        false,                              // inp_pause_enabled
        0,                                  // inp_rebill_max
        new anchor.BN(60 * 60 * 24 * 7),    // inp_max_delay
        new anchor.BN(0),                   // inp_not_valid_before
        new anchor.BN(0),                   // inp_not_valid_after
        {
            accounts: {
                subscrData: subscrData.publicKey,
                merchantKey: merchantPK.publicKey,
                userKey: provider.wallet.publicKey,
                tokenMint: tokenMint,
                tokenAccount: tokenAccount,
                rebillData: rebillData.publicKey
            }
        }
    )
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
