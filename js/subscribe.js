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
        ndjs = await fs.readFile('/Users/mfrager/Build/solana/net-authority/js/net.json')
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

    const userAgent = await programAddress([provider.wallet.publicKey.toBuffer()])

    if (false) {
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
                    { pubkey: merchantPK, isWritable: false, isSigner: false },
                    { pubkey: new PublicKey(merchantTK.pubkey), isWritable: true, isSigner: false },
                    { pubkey: TOKEN_PROGRAM_ID, isWritable: false, isSigner: false },
                    { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
                    { pubkey: SYSVAR_RENT_PUBKEY, isWritable: false, isSigner: false },
                ]
            }
        )
    }

    if (false) {
        console.log('Fund Token: Fees')
        await tokenAgent.rpc.fundToken(
            feesTK.nonce,
            {
                accounts: {
                    ascTokenAccount: SPL_ASSOCIATED_TOKEN,
                },
                remainingAccounts: [
                    { pubkey: provider.wallet.publicKey, isWritable: true, isSigner: true },
                    { pubkey: tokenMint, isWritable: false, isSigner: false },
                    { pubkey: feesPK, isWritable: false, isSigner: false },
                    { pubkey: new PublicKey(feesTK.pubkey), isWritable: true, isSigner: false },
                    { pubkey: TOKEN_PROGRAM_ID, isWritable: false, isSigner: false },
                    { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
                    { pubkey: SYSVAR_RENT_PUBKEY, isWritable: false, isSigner: false },
                ]
            }
        )
    }

    console.log('Fund Account: Subscription 1')
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
    const transactId = uuidv4()
    await tokenAgent.rpc.subscribe(
        true,
        new anchor.BN(100000),                          // initial_amount
        new anchor.BN(uuidparse(transactId)),           // initial_tx_uuid
        userAgent.nonce,                                // inp_user_nonce
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        netRoot.nonce,                                  // inp_net_nonce
        new anchor.BN(uuidparse(subscrId)),             // inp_subscr_uuid
        2,                                              // inp_period (2 = monthly)
        new anchor.BN(10000),                           // inp_budget
        new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_next_rebill
        0,                                              // inp_rebill_max
        new anchor.BN(0),                               // inp_not_valid_before
        new anchor.BN(0),                               // inp_not_valid_after
        false,                                          // inp_swap
        false,                                          // inp_swap_link
        0,                                              // inp_swap_root_nonce
        0,                                              // inp_swap_inb_nonce
        0,                                              // inp_swap_out_nonce
        {
            accounts: {
                subscrData: subscrData.publicKey,
                netAuth: netAuth,
                netRoot: new PublicKey(netRoot.pubkey),
                netRbac: netRBAC,
                rootKey: new PublicKey(rootKey.pubkey),
                merchantKey: merchantPK,
                merchantApproval: merchantAP,
                merchantToken: new PublicKey(merchantTK.pubkey),
                managerKey: managerPK,
                managerApproval: managerAP,
                userKey: provider.wallet.publicKey,
                userAgent: new PublicKey(userAgent.pubkey),
                tokenProgram: TOKEN_PROGRAM_ID,
                tokenMint: tokenMint,
                tokenAccount: tokenAccount,
                feesAccount: new PublicKey(feesTK.pubkey),
            }
        }
    )

    if (true) {
        console.log('Process 1')

        console.log({
            subscrData: subscrData.publicKey.toString(),
            merchantKey: merchantPK.toString(),
            merchantApproval: merchantAP.toString(),
            merchantToken: new PublicKey(merchantTK.pubkey).toString(),
            managerKey: managerPK.toString(),
            managerApproval: managerAP.toString(),
            userAgent: new PublicKey(userAgent.pubkey).toString(),
            tokenProgram: TOKEN_PROGRAM_ID.toString(),
            tokenMint: tokenMint.toString(),
            tokenAccount: tokenAccount.toString(),
            feesAccount: new PublicKey(feesTK.pubkey).toString(),
        })

        var eventId = uuidv4()
        var dt1 = dt0.plus({ months: 1 })
        var dts1 = dt1.toFormat("yyyyLL")
        console.log('Next Rebill: ' + dts1 + ' - ' + dt1.toISO())
        const tx3 = await tokenAgent.transaction.process(
            userAgent.nonce,                                // inp_user_nonce
            merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
            rootKey.nonce,                                  // inp_root_nonce
            netRoot.nonce,                                  // inp_net_nonce
            new anchor.BN(uuidparse(eventId)),              // inp_rebill_uuid
            new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_rebill_ts
            dts0,                                           // inp_rebill_str
            new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_next_rebill
            new anchor.BN(10000),                           // inp_amount
            0,                                              // inp_swap_root_nonce
            0,                                              // inp_swap_inb_nonce
            0,                                              // inp_swap_out_nonce
            {
                accounts: {
                    subscrData: subscrData.publicKey,
                    netAuth: netAuth,
                    netRoot: new PublicKey(netRoot.pubkey),
                    netRbac: netRBAC,
                    rootKey: new PublicKey(rootKey.pubkey),
                    merchantKey: merchantPK,
                    merchantApproval: merchantAP,
                    merchantToken: new PublicKey(merchantTK.pubkey),
                    managerKey: managerPK,
                    managerApproval: managerAP,
                    userAgent: new PublicKey(userAgent.pubkey),
                    tokenProgram: TOKEN_PROGRAM_ID,
                    tokenMint: tokenMint,
                    tokenAccount: tokenAccount,
                    feesAccount: new PublicKey(feesTK.pubkey),
                }
            }
        )
        await provider.send(tx3, [managerSK])

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
