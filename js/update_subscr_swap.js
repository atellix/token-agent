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
    const subscrData = new PublicKey('HwWdBqxZm1i9mz9Ut67XL3vcbvh151wtu2VdXhNXFmgv')

    var ndjs
    try {
        ndjs = await fs.readFile('../../data/net.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const netData = JSON.parse(ndjs.toString())
    //console.log(netData)

    var spjs
    try {
        spjs = await fs.readFile('../../data/swap.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const swapCache = JSON.parse(spjs.toString())

    var djs
    try {
        djs = await fs.readFile('../../data/swap-wsol-usdv.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const swapSpec = JSON.parse(djs.toString())

    const swapContractPK = new PublicKey(swapCache.swapContractProgram)
    const tokenMint1 = new PublicKey(swapSpec.inbMint)
    const tokenMint2 = new PublicKey(swapSpec.outMint)
    const swapAuthDataPK = new PublicKey(swapCache.swapContractRBAC)
    const swapDataPK = new PublicKey(swapSpec.swapData)
    const swapFeesTK = new PublicKey(swapSpec.feesToken)

    const swapRootData = await programAddress([swapContractPK.toBuffer()], swapContractPK)
    const tokData1 = await associatedTokenAddress(new PublicKey(swapRootData.pubkey), tokenMint1)
    const tokData2 = await associatedTokenAddress(new PublicKey(swapRootData.pubkey), tokenMint2)

    const userToken1 = await associatedTokenAddress(provider.wallet.publicKey, tokenMint1)

    const tokenMint = new PublicKey(netData.tokenMintUSDV)
    const netAuth = new PublicKey(netData.netAuthorityProgram)
    const rootKey = await programAddress([tokenAgentPK.toBuffer()])
    const netRoot = await programAddress([netAuth.toBuffer()], netAuth)
    const netRBAC = new PublicKey(netData.netAuthorityRBAC)
    const feesPK = new PublicKey(netData.fees1)
    const feesTK = await associatedTokenAddress(feesPK, tokenMint)
    const merchantPK = new PublicKey(netData.merchant1)
    const merchantTK = await associatedTokenAddress(merchantPK, tokenMint)
    const managerSK = importSecretKey(netData.manager1_secret)
    const userAgent = await programAddress([provider.wallet.publicKey.toBuffer()])

    const agentToken = await associatedTokenAddress(new PublicKey(rootKey.pubkey), tokenMint)
    const tokenAccount = new PublicKey(agentToken.pubkey)

    var act = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log('Initial Subscription Data')
    console.log(act)

    console.log('Update Subscription')
    var dt0 = DateTime.now().setZone('utc')
    //dt0 = dt0.minus({ days: dt0.day - 1, hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ months: 1 })
    dt0 = dt0.minus({ hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ days: 1 })
    var dts0 = dt0.toFormat("yyyyLLdd")
    console.log('Next Rebill: ' + dts0 + ' - ' + dt0.toISO())
    act.swap = true
    act.period = 0
    act.periodBudget = new anchor.BN(100000)
    act.useTotal = false
    act.totalBudget = new anchor.BN(0)
    act.nextRebill = new anchor.BN(Math.floor(dt0.toSeconds()))
    let txsig = await tokenAgent.rpc.updateSubscription(
        act.active,                                     // inp_active
        true,                                           // inp_link_token
        new anchor.BN(100000),                          // inp_amount
        new anchor.BN(4444),                            // inp_amount
        userAgent.nonce,                                // inp_user_nonce
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        netRoot.nonce,                                  // inp_net_nonce
        act.period,                                     // inp_period (2 = monthly)
        act.periodBudget,                               // inp_period_budget
        act.useTotal,                                   // inp_use_total
        act.totalBudget,                                // inp_total_budget
        act.nextRebill,                                 // inp_next_rebill
        act.rebillMax,                                  // inp_rebill_max
        act.notValidBefore,                             // inp_not_valid_before
        act.notValidAfter,                              // inp_not_valid_after
        act.maxDelay,                                   // inp_max_delay
        true, // act.swap,                              // inp_swap
        true, // act.swap_direction,                    // inp_swap_direction
        swapRootData.nonce,                             // inp_swap_root_nonce
        tokData1.nonce,                                 // inp_swap_inb_nonce
        tokData2.nonce,                                 // inp_swap_out_nonce
        agentToken.nonce,                               // inp_swap_dst_nonce
        {
            accounts: {
                subscrData: subscrData,
                netAuth: netAuth,
                netRoot: new PublicKey(netRoot.pubkey),
                netRbac: netRBAC,
                rootKey: new PublicKey(rootKey.pubkey),
                merchantKey: act.merchantKey,
                merchantApproval: act.merchantApproval,
                merchantToken: act.merchantToken,
                managerKey: act.managerKey,
                managerApproval: act.managerApproval,
                userKey: act.userKey,
                userAgent: act.userAgent,
                tokenProgram: TOKEN_PROGRAM_ID,
                tokenMint: act.tokenMint,
                tokenAccount: tokenAccount,
                feesAccount: new PublicKey(feesTK.pubkey),
            },
            remainingAccounts: [
                { pubkey: new PublicKey(userToken1.pubkey), isWritable: true, isSigner: false },
                { pubkey: swapContractPK, isWritable: false, isSigner: false },
                { pubkey: new PublicKey(swapRootData.pubkey), isWritable: false, isSigner: false },
                { pubkey: swapAuthDataPK, isWritable: false, isSigner: false },
                { pubkey: swapDataPK, isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tokData1.pubkey), isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tokData2.pubkey), isWritable: true, isSigner: false },
                { pubkey: swapFeesTK, isWritable: true, isSigner: false },
                { pubkey: new PublicKey('DpoK8Zz69APV9ntjuY9C4LZCxANYMV56M2cbXEdkjxME'), isWritable: false, isSigner: false },
            ],
        }
    )
    console.log(txsig)
    var act2 = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log('Updated Subscription Data')
    console.log(act2)

    var dt1
    var dts1
    var rbtx
    for (var x = 0; x < 40; x++) {
        dt1 = dt0.plus({ days: 1 })
        dts1 = dt1.toFormat("yyyyLLdd")
        console.log('Current Rebill: ' + dts0 + ' (' + Math.floor(dt0.toSeconds()) + ')')
        console.log('Next Rebill: ' + dts1 + ' - ' + dt1.toISO() + ' (' + Math.floor(dt1.toSeconds()) + ')')
        const tx3 = await tokenAgent.transaction.process(
            userAgent.nonce,                                // inp_user_nonce
            merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
            rootKey.nonce,                                  // inp_root_nonce
            netRoot.nonce,                                  // inp_net_nonce
            new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_rebill_ts
            dts0,                                           // inp_rebill_str
            new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_next_rebill
            new anchor.BN(10000),                           // inp_amount
            swapRootData.nonce,                             // inp_swap_root_nonce
            tokData1.nonce,                                 // inp_swap_inb_nonce
            tokData2.nonce,                                 // inp_swap_out_nonce
            {
                accounts: {
                    subscrData: subscrData,
                    netAuth: netAuth,
                    netRoot: new PublicKey(netRoot.pubkey),
                    netRbac: netRBAC,
                    rootKey: new PublicKey(rootKey.pubkey),
                    merchantKey: act.merchantKey,
                    merchantApproval: act.merchantApproval,
                    merchantToken: act.merchantToken,
                    managerKey: act.managerKey,
                    managerApproval: act.managerApproval,
                    userAgent: act.userAgent,
                    tokenProgram: TOKEN_PROGRAM_ID,
                    tokenMint: act.tokenMint,
                    tokenAccount: act.tokenAccount,
                    feesAccount: new PublicKey(feesTK.pubkey),
                },
                remainingAccounts: [
                    { pubkey: new PublicKey(userToken1.pubkey), isWritable: true, isSigner: false },
                    { pubkey: swapContractPK, isWritable: false, isSigner: false },
                    { pubkey: new PublicKey(swapRootData.pubkey), isWritable: false, isSigner: false },
                    { pubkey: swapAuthDataPK, isWritable: false, isSigner: false },
                    { pubkey: swapDataPK, isWritable: true, isSigner: false },
                    { pubkey: new PublicKey(tkiData1.pubkey), isWritable: true, isSigner: false },
                    { pubkey: new PublicKey(tokData1.pubkey), isWritable: true, isSigner: false },
                    { pubkey: new PublicKey(tkiData2.pubkey), isWritable: true, isSigner: false },
                    { pubkey: new PublicKey(tokData2.pubkey), isWritable: true, isSigner: false },
                    { pubkey: swapFeesTK, isWritable: true, isSigner: false },
                    { pubkey: new PublicKey('DpoK8Zz69APV9ntjuY9C4LZCxANYMV56M2cbXEdkjxME'), isWritable: false, isSigner: false },
                ],
            }
        )
        rbtx = await provider.send(tx3, [managerSK])
        console.log(rbtx)
        dt0 = dt1
        dts0 = dts1
    }
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
