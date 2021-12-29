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

    console.log('Update Subscription - No funding')
    /*var dt0 = DateTime.now().setZone('utc')
    dt0 = dt0.minus({ days: dt0.day - 1, hours: dt0.hour, minutes: dt0.minute, seconds: dt0.second }).plus({ months: 1 })
    var dts0 = dt0.toFormat("yyyyLL")
    console.log('Next Rebill: ' + dts0 + ' - ' + dt0.toISO())*/
    await tokenAgent.rpc.updateSubscription(
        true,                                           // link_token
        new anchor.BN(100000),                          // initial_amount
        userAgent.nonce,                                // inp_user_nonce
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        netRoot.nonce,                                  // inp_net_nonce
        new anchor.BN(0),                               // inp_subscr_id
        2,                                              // inp_period (2 = monthly)
        new anchor.BN(10000),                           // inp_budget
        new anchor.BN(Math.floor(dt0.toSeconds())),     // inp_next_rebill
        false,                                          // inp_swap
        0,                                              // inp_swap_root_nonce
        0,                                              // inp_swap_inb_nonce
        0,                                              // inp_swap_out_nonce
        0,                                              // inp_swap_dst_nonce
        {
            accounts: {
                //subscrData: new PublicKey('Fxg4sFxmiWFPaxS7Xtgnk4J83grzcky9ZpMd6GyutEPd'),
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
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
