const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
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
    var ndjs
    try {
        ndjs = await fs.readFile('../../data/net.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const netData = JSON.parse(ndjs.toString())

    var swidl
    try {
        swidl = await fs.readFile('../../swap-contract/target/idl/swap_contract.json')
    } catch (error) {
        console.error('File Error: ', error)
    }
    const swapContractIDL = JSON.parse(swidl.toString())

    //console.log(netData)
    const netAuth = new PublicKey(netData.netAuthorityProgram)
    const tokenMint = new PublicKey(netData.tokenMintUSDV)
    const walletToken = await associatedTokenAddress(provider.wallet.publicKey, tokenMint)
    //const tokenAccount = new PublicKey(walletToken.pubkey)

    const rootKey = await programAddress([tokenAgentPK.toBuffer()])
    const netRoot = await programAddress([netAuth.toBuffer()], netAuth)
    const netRBAC = new PublicKey(netData.netAuthorityRBAC)

    const merchantAP = new PublicKey(netData.merchantApproval1)
    const merchantPK = new PublicKey(netData.merchant1_dest)
    const merchantTK = await associatedTokenAddress(merchantPK, tokenMint)
    const feesPK = new PublicKey(netData.fees1)
    const feesTK = await associatedTokenAddress(feesPK, tokenMint)

    console.log('Token Account Mint: ' + tokenMint.toString())
    console.log('Token Account Owner: ' + provider.wallet.publicKey.toString())

    console.log('Merchant Account: ' + merchantPK.toString())
    console.log('Merchant Token: ' + merchantTK.pubkey)

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

    const swapId = 0
    var buf = Buffer.alloc(2)
    buf.writeInt16LE(swapId)
    const swapData = await programAddress([tokenMint1.toBuffer(), tokenMint2.toBuffer(), buf], swapContractPK)
    const tokData1 = await associatedTokenAddress(new PublicKey(swapData.pubkey), tokenMint1)
    const tokData2 = await associatedTokenAddress(new PublicKey(swapData.pubkey), tokenMint2)

    const userToken1 = await associatedTokenAddress(provider.wallet.publicKey, tokenMint1)

    const agentToken = await associatedTokenAddress(new PublicKey(rootKey.pubkey), tokenMint)
    const tokenAccount = new PublicKey(agentToken.pubkey)
    console.log('Token Account Assoc: ' + tokenAccount.toString())

    console.log('User Token 1: ' + userToken1.pubkey)
    console.log('Payment Token: ' + tokenAccount.toString())

    const swapContract = new anchor.Program(swapContractIDL, swapContractPK)
    var l1 = swapContract.addEventListener('SwapEvent', (evt, slot) => {
        console.log('Event - Slot: ' + slot)
        console.log(evt.eventHash.toString())
        console.log(evt)
    })

    var l2 = tokenAgent.addEventListener('PaymentEvent', (evt, slot) => {
        console.log('Event - Slot: ' + slot)
        console.log(evt.eventHash.toString())
        console.log(evt)
    })

    const transactId = uuidv4()
    console.log('Merchant Payment: ' + transactId)
    console.log([
        {
            netAuth: netAuth.toString(),
            rootKey: new PublicKey(rootKey.pubkey).toString(),
            merchantKey: merchantPK.toString(),
            merchantApproval: merchantAP.toString(),
            merchantToken: new PublicKey(merchantTK.pubkey).toString(),
            userKey: provider.wallet.publicKey.toString(),
            tokenProgram: TOKEN_PROGRAM_ID.toString(),
            tokenMint: tokenMint.toString(),
            tokenAccount: tokenAccount.toString(),
            feesAccount: new PublicKey(feesTK.pubkey).toString(),
        },
        [
            { pubkey: new PublicKey(userToken1.pubkey).toString() },
            { pubkey: swapContractPK.toString() },
            { pubkey: new PublicKey(swapData.pubkey).toString() },
            { pubkey: new PublicKey(tokData1.pubkey).toString() },
            { pubkey: new PublicKey(tokData2.pubkey).toString() },
            { pubkey: swapFeesTK.toString() },
            { pubkey: new PublicKey('GvDMxPzN1sCj7L26YDK2HnMRXEQmQ2aemov8YBtPS7vR').toString() },
        ],
    ])

    let apires = await tokenAgent.rpc.merchantPayment(
        merchantTK.nonce,                               // inp_merchant_nonce (merchant associated token account nonce)
        rootKey.nonce,                                  // inp_root_nonce
        new anchor.BN(12345),                           // inp_payment_id
        new anchor.BN(20 * (10**4)),                    // inp_amount
        true,                                           // inp_swap
        true,                                           // inp_swap_direction
        0,                                              // inp_swap_mode: 0 = AtxSwapContractV1
        swapData.nonce,                                 // inp_swap_data_nonce
        tokData1.nonce,                                 // inp_swap_inb_nonce
        tokData2.nonce,                                 // inp_swap_out_nonce
        agentToken.nonce,                               // inp_swap_dst_nonce
        {
            accounts: {
                netAuth: netAuth,
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
            remainingAccounts: [
                { pubkey: new PublicKey(userToken1.pubkey), isWritable: true, isSigner: false },
                { pubkey: swapContractPK, isWritable: false, isSigner: false },
                { pubkey: new PublicKey(swapData.pubkey), isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tokData1.pubkey), isWritable: true, isSigner: false },
                { pubkey: new PublicKey(tokData2.pubkey), isWritable: true, isSigner: false },
                { pubkey: swapFeesTK, isWritable: true, isSigner: false },
                { pubkey: new PublicKey('GvDMxPzN1sCj7L26YDK2HnMRXEQmQ2aemov8YBtPS7vR'), isWritable: false, isSigner: false },
            ],
        }
    )
    console.log(apires)
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
