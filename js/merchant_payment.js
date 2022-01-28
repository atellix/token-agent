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
    console.log(netData)
    const netAuth = new PublicKey(netData.netAuthorityProgram)
    const tokenMint = new PublicKey(netData.tokenMintUSDV)
    const walletToken = await associatedTokenAddress(provider.wallet.publicKey, tokenMint)
    const tokenAccount = new PublicKey(walletToken.pubkey)

    const rootKey = await programAddress([tokenAgentPK.toBuffer()])
    const netRoot = await programAddress([netAuth.toBuffer()], netAuth)
    const netRBAC = new PublicKey(netData.netAuthorityRBAC)

    const merchantPK = new PublicKey(netData.merchant1)
    const merchantAP = new PublicKey(netData.merchantApproval1)
    const merchantTK = await associatedTokenAddress(merchantPK, tokenMint)
    const feesPK = new PublicKey(netData.fees1)
    const feesTK = await associatedTokenAddress(feesPK, tokenMint)

    console.log('Token Account Mint: ' + tokenMint.toString())
    console.log('Token Account Owner: ' + provider.wallet.publicKey.toString())
    console.log('Token Account Assoc: ' + tokenAccount.toString())

    console.log('Merchant Account: ' + merchantPK.toString())
    console.log('Merchant Token: ' + merchantTK.pubkey)

/*    var spjs
    try {
        spjs = await fs.readFile('../../data/swap.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const swapCache = JSON.parse(spjs.toString())

    var djs
    try {
        djs = await fs.readFile('../../data/swap-usdv-wsol.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const swapSpec = JSON.parse(djs.toString())

    const swapContractPK = new PublicKey(swapCache.swapContractProgram)
    const tokenMint1 = new PublicKey(swapSpec.tokenMint1)
    const tokenMint2 = new PublicKey(swapSpec.tokenMint2)
    const swapAuthDataPK = new PublicKey(swapCache.swapContractRBAC)
    const swapDataPK = new PublicKey(swapSpec.swapData)
    const swapFeesTK = new PublicKey(swapSpec.feesToken)

    const swapRootData = await programAddress([swapContractPK.toBuffer()], swapContractPK)
    const tkiData1 = await programAddress([tokenMint1.toBuffer()], swapContractPK)
    const tkiData2 = await programAddress([tokenMint2.toBuffer()], swapContractPK)
    const tokData1 = await associatedTokenAddress(new PublicKey(swapRootData.pubkey), tokenMint1)
    const tokData2 = await associatedTokenAddress(new PublicKey(swapRootData.pubkey), tokenMint2)

    const userToken1 = await associatedTokenAddress(provider.wallet.publicKey, tokenMint1)

    console.log('User Token 1: ' + userToken1.pubkey)
    console.log('Payment Token: ' + tokenAccount.toString()) */

    var l1 = tokenAgent.addEventListener('PaymentEvent', (evt, slot) => {
        console.log('PaymentEvent - Slot: ' + slot)
        console.log(evt.eventHash.toString())
        console.log(evt)
    })

    const transactId = uuidv4()
    console.log('Merchant Payment: ' + transactId)
    let apires = await tokenAgent.rpc.merchantPayment(
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        netRoot.nonce,                                  // inp_net_nonce
        new anchor.BN(1234),                            // inp_payment_id
        new anchor.BN(20 * (10**4)),                    // inp_amount
        false,                                          // inp_swap
        0,                                              // inp_swap_root_nonce
        0,                                              // inp_swap_inb_nonce
        0,                                              // inp_swap_out_nonce
        0,                                              // inp_swap_dst_nonce
        {
            accounts: {
                netAuth: netAuth,
                netRoot: new PublicKey(netRoot.pubkey),
                netRbac: netRBAC,
                rootKey: new PublicKey(rootKey.pubkey),
                merchantKey: merchantPK,
                merchantApproval: merchantAP,
                merchantToken: new PublicKey(merchantTK.pubkey),
                userKey: provider.wallet.publicKey,
                tokenProgram: TOKEN_PROGRAM_ID,
                tokenMint: tokenMint,
                tokenAccount: tokenAccount,
                feesAccount: new PublicKey(feesTK.pubkey),
            },
            /*remainingAccounts: [
                { pubkey: new PublicKey(userToken1.pubkey), isWritable: true, isSigner: false },
                { pubkey: swapContractPK, isWritable: false, isSigner: false },
                { pubkey: new PublicKey(swapRootData.pubkey), isWritable: false, isSigner: false },
                { pubkey: swapAuthDataPK, isWritable: false, isSigner: false },
                { pubkey: provider.wallet.publicKey, isWritable: false, isSigner: true },
                { pubkey: swapDataPK, isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tkiData1.pubkey), isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tokData1.pubkey), isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tkiData2.pubkey), isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tokData2.pubkey), isWritable: true, isSigner: false },
                { pubkey: swapFeesTK, isWritable: true, isSigner: false },
                { pubkey: new PublicKey('DpoK8Zz69APV9ntjuY9C4LZCxANYMV56M2cbXEdkjxME'), isWritable: false, isSigner: false },
            ],*/
        }
    )
    console.log(apires)
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
