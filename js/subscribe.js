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

const SPL_ASSOCIATED_TOKEN = new PublicKey('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL')
async function associatedTokenAddress(walletAddress, tokenMintAddress) {
    const addr = await PublicKey.findProgramAddress(
        [walletAddress.toBuffer(), TOKEN_PROGRAM_ID.toBuffer(), tokenMintAddress.toBuffer()],
        SPL_ASSOCIATED_TOKEN
    )
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

async function programAddress(inputs, programPK = tokenAgentPK) {
    const addr = await PublicKey.findProgramAddress(inputs, programPK)
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

function importSecretKey(keyStr) {
    var dec = new base32.Decoder({ type: "crockford" })
    var spec = dec.write(keyStr).finalize()
    return Keypair.fromSecretKey(new Uint8Array(spec))
}

async function main() {
    var ndjs
    try {
        ndjs = await fs.readFile('../../data/net.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const netData = JSON.parse(ndjs.toString())
    //console.log(netData)
    const netAuth = new PublicKey(netData.netAuthorityProgram)
    const tokenMint = new PublicKey(netData.tokenMintUSDV)
    const walletToken = await associatedTokenAddress(provider.wallet.publicKey, tokenMint)
    const tokenAccount = new PublicKey(walletToken.pubkey)

    const rootKey = await programAddress([tokenAgentPK.toBuffer()])
    const netRoot = await programAddress([netAuth.toBuffer()], netAuth)
    const netRBAC = new PublicKey(netData.netAuthorityRBAC)

    const subscrId = uuidv4()
    const subscrData = anchor.web3.Keypair.generate()
    const subscrDataBytes = tokenAgent.account.subscrData.size
    console.log('Subscr Data Bytes: ' + subscrDataBytes)
    const subscrDataRent = await provider.connection.getMinimumBalanceForRentExemption(subscrDataBytes)
    console.log('Subscr Data Rent: ' + subscrDataRent)
    //const merchantPK = anchor.web3.Keypair.generate()
    //const merchantAP = anchor.web3.Keypair.generate()
    const merchantPK = new PublicKey(netData.merchant1)
    const merchantAP = new PublicKey(netData.merchantApproval1)
    const merchantTK = await associatedTokenAddress(merchantPK, tokenMint)
    //const managerPK = anchor.web3.Keypair.generate()
    //const managerAP = anchor.web3.Keypair.generate()
    const managerPK = new PublicKey(netData.manager1)
    const managerSK = importSecretKey(netData.manager1_secret)
    const managerAP = new PublicKey(netData.managerApproval1)
    const feesPK = new PublicKey(netData.fees1)
    const feesTK = await associatedTokenAddress(feesPK, tokenMint)

    console.log('Token Account Mint: ' + tokenMint.toString())
    console.log('Token Account Owner: ' + provider.wallet.publicKey.toString())
    console.log('Token Account Assoc: ' + tokenAccount.toString())

    console.log('Merchant Account: ' + merchantPK.toString())
    console.log('Merchant Token: ' + merchantTK.pubkey)
    console.log('Subscription Data: ' + subscrData.publicKey.toString())

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

    var l1 = tokenAgent.addEventListener('SubscrEvent', (evt, slot) => {
        console.log('SubscrEvent - Slot: ' + slot)
        console.log(evt.eventHash.toString())
        console.log(evt)
    })

    console.log('Subscribe')

    console.log({
        subscrData: subscrData.publicKey.toString(),
        netAuth: netAuth.toString(),
        rootKey: new PublicKey(rootKey.pubkey).toString(),
        merchantApproval: merchantAP.toString(),
        merchantToken: new PublicKey(merchantTK.pubkey).toString(),
        managerApproval: managerAP.toString(),
        userKey: provider.wallet.publicKey.toString(),
        tokenProgram: TOKEN_PROGRAM_ID.toString(),
        tokenAccount: tokenAccount.toString(),
        feesAccount: new PublicKey(feesTK.pubkey).toString(),
        systemProgram: SystemProgram.programId.toString(),
    })

    var dt0 = DateTime.now().setZone('utc')
    dt0 = dt0.minus({ days: dt0.day - 1, hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ months: 1 })
    var dts0 = dt0.toFormat("yyyyLL")
    console.log('Next Rebill: ' + dts0 + ' - ' + dt0.toISO())
    tx.add(tokenAgent.instruction.subscribe(
        true,                                           // link_token
        new anchor.BN(100000),                          // initial_amount
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        new anchor.BN(777),                             // inp_subscr_id
        new anchor.BN(888),                             // inp_payment_id
        2,                                              // inp_period (2 = monthly)
        new anchor.BN(10000),                           // inp_budget
        false,                                          // inp_use_total
        new anchor.BN(0),                               // inp_total_budget
        new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_next_rebill
        0,                                              // inp_rebill_max
        new anchor.BN(0),                               // inp_not_valid_before
        new anchor.BN(0),                               // inp_not_valid_after
        new anchor.BN(0),                               // inp_max_delay
        false,                                          // inp_swap
        false,                                          // inp_swap_direction
        0,                                              // inp_swap_mode
        0,                                              // inp_swap_root_nonce
        0,                                              // inp_swap_inb_nonce
        0,                                              // inp_swap_out_nonce
        0,                                              // inp_swap_dst_nonce
        {
            accounts: {
                subscrData: subscrData.publicKey,
                netAuth: netAuth,
                rootKey: new PublicKey(rootKey.pubkey),
                merchantApproval: merchantAP,
                merchantToken: new PublicKey(merchantTK.pubkey),
                managerApproval: managerAP,
                userKey: provider.wallet.publicKey,
                tokenProgram: TOKEN_PROGRAM_ID,
                tokenAccount: tokenAccount,
                feesAccount: new PublicKey(feesTK.pubkey),
                systemProgram: SystemProgram.programId,
            },
        }
    ))
    let txid = await provider.send(tx, [subscrData])
    console.log(txid)

    if (true) {
        console.log('Process 1')

        console.log({
            subscrData: subscrData.publicKey.toString(),
            merchantKey: merchantPK.toString(),
            merchantApproval: merchantAP.toString(),
            merchantToken: new PublicKey(merchantTK.pubkey).toString(),
            managerKey: managerPK.toString(),
            managerApproval: managerAP.toString(),
            tokenProgram: TOKEN_PROGRAM_ID.toString(),
            tokenMint: tokenMint.toString(),
            tokenAccount: tokenAccount.toString(),
            feesAccount: new PublicKey(feesTK.pubkey).toString(),
        })

        var dt1 = dt0.plus({ months: 1 })
        var dts1 = dt1.toFormat("yyyyLL")
        console.log('Next Rebill: ' + dts1 + ' - ' + dt1.toISO())
        const tx3 = await tokenAgent.transaction.process(
            merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
            rootKey.nonce,                                  // inp_root_nonce
            new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_rebill_ts
            dts0,                                           // inp_rebill_str
            new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_next_rebill
            new anchor.BN(10000),                           // inp_amount
            new anchor.BN(121212),                          // inp_payment_id
            0,                                              // inp_swap_root_nonce
            0,                                              // inp_swap_inb_nonce
            0,                                              // inp_swap_out_nonce
            {
                accounts: {
                    subscrData: subscrData.publicKey,
                    netAuth: netAuth,
                    rootKey: new PublicKey(rootKey.pubkey),
                    merchantApproval: merchantAP,
                    merchantToken: new PublicKey(merchantTK.pubkey),
                    managerKey: managerPK,
                    managerApproval: managerAP,
                    tokenProgram: TOKEN_PROGRAM_ID,
                    tokenAccount: tokenAccount,
                    feesAccount: new PublicKey(feesTK.pubkey),
                }
            }
        )
        console.log(await provider.send(tx3, [managerSK]))

        /* console.log('Process 2')
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
        await provider.send(tx4, [managerPK]) */
    }
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
