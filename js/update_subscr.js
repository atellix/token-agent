const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { Keypair, PublicKey, SystemProgram, Transaction, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')
const fs = require('fs').promises
const base32 = require("base32.js")

const anchor = require('@project-serum/anchor')
const provider = anchor.AnchorProvider.env()
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
    const subscrData = new PublicKey('7y3nHg2k5EEUKSeHZBGmMgbKg7QJuFqDD22HFH94qi3G')

    var ndjs
    try {
        ndjs = await fs.readFile('../../data/net.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const netData = JSON.parse(ndjs.toString())
    //console.log(netData)
    const tokenMint = new PublicKey(netData.tokenMintUSDV)
    const tokenAccount = await associatedTokenAddress(provider.wallet.publicKey, tokenMint)
    const tokenAccountPK = new PublicKey(tokenAccount.pubkey)
    const netAuth = new PublicKey(netData.netAuthorityProgram)
    const rootKey = await programAddress([tokenAgentPK.toBuffer()], tokenAgentPK)
    const rootKeyPK = new PublicKey(rootKey.pubkey)
    const netRoot = await programAddress([netAuth.toBuffer()], netAuth)
    const netRBAC = new PublicKey(netData.netAuthorityRBAC)
    const feesPK = new PublicKey(netData.fees1)
    const feesTK = await associatedTokenAddress(feesPK, tokenMint)
    const merchantPK = new PublicKey(netData.merchant1)
    const merchantTK = await associatedTokenAddress(merchantPK, tokenMint)
    const merchantToken = await associatedTokenAddress(merchantPK, tokenMint)
    const managerSK = importSecretKey(netData.manager1_secret)

    const delegateProgram = new PublicKey('TDLGbdMdskdC2DPz2eSeW3tuxtqRchjt5JMsUrdGTGm')
    const delegateRoot = await programAddress([delegateProgram.toBuffer()], delegateProgram)
    const delegateRootPK = new PublicKey(delegateRoot.pubkey)
    const allowance = await programAddress([tokenAccountPK.toBuffer(), rootKeyPK.toBuffer()], delegateProgram)
    const allowancePK = new PublicKey(allowance.pubkey)

    var act = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log('Initial Subscription Data')
    console.log(act)

    console.log('Update Subscription')
    var dt0 = DateTime.now().setZone('utc')
    //dt0 = dt0.minus({ days: dt0.day - 1, hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ months: 1 })
    dt0 = dt0.minus({ hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ days: 1 })
    var dts0 = dt0.toFormat("yyyyLLdd")
    console.log('Next Rebill: ' + dts0 + ' - ' + dt0.toISO())
    act.period = 0
    act.periodBudget = new anchor.BN(2250000)
    act.useTotal = false
    act.totalBudget = new anchor.BN(0)
    act.nextRebill = new anchor.BN(Math.floor(dt0.toSeconds()))
    let tx = new Transaction()
    tx.add(tokenAgent.transaction.updateSubscription(
        act.active,                                     // inp_active
        true,                                           // inp_link_token
        act.maxDelay,                                   // inp_max_delay
        act.nextRebill,                                 // inp_next_rebill
        act.notValidBefore,                             // inp_not_valid_before
        act.notValidAfter,                              // inp_not_valid_after
        new anchor.BN(100000),                          // inp_amount
        new anchor.BN(uuidparse(uuidv4())),             // inp_payment_id
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        act.period,                                     // inp_period (2 = monthly)
        act.periodBudget,                               // inp_period_budget
        act.useTotal,                                   // inp_use_total
        act.totalBudget,                                // inp_total_budget
        act.rebillMax,                                  // inp_rebill_max
        false, // act.swap,                             // inp_swap
        false, // act.swap_direction,                   // inp_swap_direction
        0,                                              // inp_swap_mode
        0,                                              // inp_swap_data_nonce
        0,                                              // inp_swap_inb_nonce
        0,                                              // inp_swap_out_nonce
        0,                                              // inp_swap_dst_nonce
        {
            accounts: {
                subscrData: subscrData,
                netAuth: netAuth,
                netRoot: new PublicKey(netRoot.pubkey),
                netRbac: netRBAC,
                rootKey: new PublicKey(rootKey.pubkey),
                merchantKey: act.merchantKey,
                merchantApproval: act.merchantApproval,
                merchantToken: new PublicKey(merchantToken.pubkey),
                managerKey: act.managerKey,
                managerApproval: act.managerApproval,
                userKey: act.userKey,
                tokenProgram: TOKEN_PROGRAM_ID,
                tokenMint: act.tokenMint,
                tokenAccount: new PublicKey(tokenAccount.pubkey),
                feesAccount: new PublicKey(feesTK.pubkey),
                delegateProgram: delegateProgram,
                delegateRoot: delegateRootPK,
                allowance: allowancePK,
                systemProgram: SystemProgram.programId,
            }
        }
    ))
    var txsig = await provider.sendAndConfirm(tx, [], { 'skipPreflight': true })
    console.log(txsig)
    var act2 = await tokenAgent.account.subscrData.fetch(subscrData)
    console.log('Updated Subscription Data')
    console.log(act2)

    var dt1
    var dts1
    var rbtx
    for (var x = 0; x < 3; x++) {
        dt1 = dt0.plus({ days: 1 })
        dts1 = dt1.toFormat("yyyyLLdd")
        console.log('Current Rebill: ' + dts0 + ' (' + Math.floor(dt0.toSeconds()) + ')')
        console.log('Next Rebill: ' + dts1 + ' - ' + dt1.toISO() + ' (' + Math.floor(dt1.toSeconds()) + ')')
        const tx3 = await tokenAgent.transaction.process(
            merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
            rootKey.nonce,                                  // inp_root_nonce
            new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_rebill_ts
            dts0,                                           // inp_rebill_str
            new anchor.BN(Math.floor(dt1.toSeconds())),     // inp_next_rebill
            new anchor.BN(10000),                           // inp_amount
            new anchor.BN(38483483),                        // inp_payment_id
            0,                                              // inp_swap_root_nonce
            0,                                              // inp_swap_inb_nonce
            0,                                              // inp_swap_out_nonce
            new anchor.BN(0),                               // inp_swap_estimate
            {
                accounts: {
                    subscrData: subscrData,
                    netAuth: netAuth,
                    netRoot: new PublicKey(netRoot.pubkey),
                    netRbac: netRBAC,
                    rootKey: new PublicKey(rootKey.pubkey),
                    merchantKey: act.merchantKey,
                    merchantApproval: act.merchantApproval,
                    merchantToken: new PublicKey(merchantToken.pubkey),
                    managerKey: act.managerKey,
                    managerApproval: act.managerApproval,
                    tokenProgram: TOKEN_PROGRAM_ID,
                    tokenMint: act.tokenMint,
                    tokenAccount: new PublicKey(tokenAccount.pubkey),
                    feesAccount: new PublicKey(feesTK.pubkey),
                    delegateProgram: delegateProgram,
                    delegateRoot: delegateRootPK,
                    allowance: allowancePK,
                }
            }
        )
        rbtx = await provider.sendAndConfirm(tx3, [managerSK])
        console.log(rbtx)
        dt0 = dt1
        dts0 = dts1
    }
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
