const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { PublicKey } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')

const anchor = require('@project-serum/anchor')
//const provider = anchor.Provider.env()
const provider = anchor.Provider.local()
anchor.setProvider(provider)
const tokenAgent = anchor.workspace.TokenAgent
const tokenAgentPK = tokenAgent.programId
//console.log(tokenAgent)

async function main() {
    const subscrId = uuidv4()
    const subscrData = anchor.web3.Keypair.generate()
    const subscrDataBytes = tokenAgent.account.subscrData.size
    const subscrDataRent = await provider.connection.getMinimumBalanceForRentExemption(subscrDataBytes)
    console.log('Subscr Data Rent: ' + subscrDataRent)
    const merchantPK = anchor.web3.Keypair.generate()
    const merchantAP = anchor.web3.Keypair.generate()
    const managerPK = anchor.web3.Keypair.generate()
    const managerAP = anchor.web3.Keypair.generate()
    const tokenMint = new PublicKey('7KCJVP436UCWf4qT4Nc6ora62ZqsYtadyft47QLmFUHL')
    const tokenAccount = new PublicKey('44EkCqNcJJZA7h5aaPDTnmj1cuLgigdxUbXdhpBX58nk')
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

    console.log('Subscribe')
    var dt0 = DateTime.now().setZone('utc')
    dt0 = dt0.minus({ days: dt0.day - 1, hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ months: 1 })
    var dts0 = dt0.toFormat("yyyyLL")
    console.log('Next Rebill: ' + dts0 + ' - ' + dt0.toISO())
    await tokenAgent.rpc.createSubscription(
        new anchor.BN(uuidparse(subscrId)),             // inp_subscr_uuid
        2,                                              // inp_period (2 = monthly)
        new anchor.BN(10000),                           // inp_budget
        new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_next_rebill
        false,                                          // inp_pause_enabled
        0,                                              // inp_rebill_max
        new anchor.BN(0),                               // inp_not_valid_before
        new anchor.BN(0),                               // inp_not_valid_after
        {
            accounts: {
                subscrData: subscrData.publicKey,
                merchantKey: merchantPK.publicKey,
                merchantApproval: merchantAP.publicKey,
                managerKey: managerPK.publicKey,
                managerApproval: managerAP.publicKey,
                userKey: provider.wallet.publicKey,
                tokenMint: tokenMint,
                tokenAccount: tokenAccount
            }
        }
    )

    console.log('Process 1')
    var eventId = uuidv4()
    var dt1 = dt0.plus({ months: 1 })
    var dts1 = dt1.toFormat("yyyyLL")
    console.log('Next Rebill: ' + dts1 + ' - ' + dt1.toISO())
    const tx3 = await tokenAgent.transaction.processSubscription(
        new anchor.BN(uuidparse(eventId)),              // inp_event_uuid
        new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_rebill_ts
        dts0,                                           // inp_rebill_str
        new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_next_rebill
        new anchor.BN(5000),                            // inp_amount
        {
            accounts: {
                subscrData: subscrData.publicKey,
                managerKey: managerPK.publicKey,
                managerApproval: managerAP.publicKey
            }
        }
    )
    await provider.send(tx3, [managerPK])

    console.log('Process 2')
    eventId = uuidv4()
    dt1 = dt0.plus({ months: 1 })
    dts1 = dt1.toFormat("yyyyLL")
    console.log('Next Rebill: ' + dts1 + ' - ' + dt1.toISO())
    const tx4 = await tokenAgent.transaction.processSubscription(
        new anchor.BN(uuidparse(eventId)),              // inp_event_uuid
        new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_rebill_ts
        dts0,                                           // inp_rebill_str
        new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_next_rebill
        new anchor.BN(5000),                            // inp_amount
        {
            accounts: {
                subscrData: subscrData.publicKey,
                managerKey: managerPK.publicKey,
                managerApproval: managerAP.publicKey
            }
        }
    )
    await provider.send(tx4, [managerPK])

}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
