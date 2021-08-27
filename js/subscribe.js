const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')

const anchor = require('@project-serum/anchor')
//const provider = anchor.Provider.env()
const provider = anchor.Provider.local()
anchor.setProvider(provider)
const tokenAgent = anchor.workspace.TokenAgent
const tokenAgentPK = tokenAgent.programId
//console.log(tokenAgent)

const SPL_ASSOCIATED_TOKEN = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL')
async function associatedTokenAddress(walletAddress, tokenMintAddress) {
    const addr = await PublicKey.findProgramAddress(
        [walletAddress.toBuffer(), TOKEN_PROGRAM_ID.toBuffer(), tokenMintAddress.toBuffer()],
        SPL_ASSOCIATED_TOKEN
    )
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

async function programAddress(inputs) {
    const addr = await PublicKey.findProgramAddress(inputs, tokenAgentPK)
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

async function main() {
    const tokenMint = new PublicKey('7KCJVP436UCWf4qT4Nc6ora62ZqsYtadyft47QLmFUHL')
    const tokenAccount = new PublicKey('44EkCqNcJJZA7h5aaPDTnmj1cuLgigdxUbXdhpBX58nk')

    const subscrId = uuidv4()
    const subscrData = anchor.web3.Keypair.generate()
    const subscrDataBytes = tokenAgent.account.subscrData.size
    const subscrDataRent = await provider.connection.getMinimumBalanceForRentExemption(subscrDataBytes)
    console.log('Subscr Data Rent: ' + subscrDataRent)
    const merchantPK = anchor.web3.Keypair.generate()
    const merchantAP = anchor.web3.Keypair.generate()
    const merchantTK = await associatedTokenAddress(merchantPK.publicKey, tokenMint)
    const managerPK = anchor.web3.Keypair.generate()
    const managerAP = anchor.web3.Keypair.generate()

    console.log('Fund Token: Merchant')
    await tokenAgent.rpc.fundToken(
        merchantTK.nonce,
        {
            accounts: {
                ascTokenAccount: SPL_ASSOCIATED_TOKEN,
            },
            remainingAccounts: [
                { pubkey: provider.wallet.publicKey, isWritable: true, isSigner: true },
                { pubkey: tokenMint, isWritable: false, isSigner: false },
                { pubkey: merchantPK.publicKey, isWritable: false, isSigner: false },
                { pubkey: new PublicKey(merchantTK.pubkey), isWritable: true, isSigner: false },
                { pubkey: TOKEN_PROGRAM_ID, isWritable: false, isSigner: false },
                { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
                { pubkey: SYSVAR_RENT_PUBKEY, isWritable: false, isSigner: false },
            ]
        }
    )

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
    const programDA = await programAddress([provider.wallet.publicKey.toBuffer()])
    var dt0 = DateTime.now().setZone('utc')
    dt0 = dt0.minus({ days: dt0.day - 1, hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ months: 1 })
    var dts0 = dt0.toFormat("yyyyLL")
    console.log('Next Rebill: ' + dts0 + ' - ' + dt0.toISO())
    await tokenAgent.rpc.subscribe(
        true,
        new anchor.BN(1000 * 1000000),
        programDA.nonce,
        merchantTK.nonce,
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
                merchantToken: new PublicKey(merchantTK.pubkey),
                managerKey: managerPK.publicKey,
                managerApproval: managerAP.publicKey,
                userKey: provider.wallet.publicKey,
                userAgent: new PublicKey(programDA.pubkey),
                tokenProgram: TOKEN_PROGRAM_ID,
                tokenMint: tokenMint,
                tokenAccount: tokenAccount,
                tokenAgent: tokenAgentPK
            }
        }
    )

    if (false) {
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
        dt2 = dt1.plus({ months: 1 })
        dts2 = dt2.toFormat("yyyyLL")
        console.log('Next Rebill: ' + dts2 + ' - ' + dt2.toISO())
        const tx4 = await tokenAgent.transaction.processSubscription(
            new anchor.BN(uuidparse(eventId)),              // inp_event_uuid
            new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_rebill_ts
            dts1,                                           // inp_rebill_str
            new anchor.BN(Math.floor(dt2.toSeconds())),     // inp_next_rebill
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
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
