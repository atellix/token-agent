const { Buffer } = require('buffer')
const { DateTime } = require("luxon")
const { v4: uuidv4, parse: uuidparse } = require('uuid')
const { Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } = require('@solana/web3.js')
const { TOKEN_PROGRAM_ID } = require('@solana/spl-token')
const base32 = require("base32.js")
const { importSecretKey, exportSecretKey } = require('../../js/atellix-common')

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

async function programAddress(inputs) {
    const addr = await PublicKey.findProgramAddress(inputs, tokenAgentPK)
    const res = { 'pubkey': await addr[0].toString(), 'nonce': addr[1] }
    return res
}

async function main() {
    const tokenMint = new PublicKey('5dH5PLd8VRSbdo5K9Ftkf2Bsqbmre3raErZYMB3vYDww')
    const tokenAccount = new PublicKey('27ik9WP4p85Nhgoy7Us49pJ1JDAcR22hnbteSBdfuqHE')
    const tokenRecipient = new PublicKey('3Qh11vYtpKamT8WK8mD5UgdeBCk3gP8Z5eh3oe98AsGC')
    const tokenRecipient2 = new PublicKey('ETvRsP3BgBSJ3MtApN3WAs9asYH4ktj6BCZEi4xu8Txa')

    //const subscrId = uuidv4()
    //const subscrData = anchor.web3.Keypair.generate()
    //const subscrDataBytes = tokenAgent.account.subscrData.size
    //const subscrDataRent = await provider.connection.getMinimumBalanceForRentExemption(subscrDataBytes)
    //console.log('Subscr Data Rent: ' + subscrDataRent)
    var merchantPK = anchor.web3.Keypair.generate().publicKey
    //merchantPK = new PublicKey('9GwKZ3yGxmAvh4kaAi18B5wh3keFjwX63hA7oCK9iqXZ')
    //const merchantAP = anchor.web3.Keypair.generate()
    const merchantTK = await associatedTokenAddress(merchantPK, tokenMint)
    var managerPK
    if (false) {
        managerPK = anchor.web3.Keypair.generate()
    } else {
        //managerPK = anchor.web3.Keypair.fromSecretKey(new Uint8Array([])
        managerPK = importSecretKey('3h9y0bjxfj204gsf5rr9913hpkvwyvreatp65tc89mb7p7cm7wk250d9k60p3q9jgv65azenq853wfp1zztccbr5h4jyc97d7g13138')
    }
    //const managerAP = anchor.web3.Keypair.generate()

    const userAgent = await programAddress([provider.wallet.publicKey.toBuffer()])
    let allowanceSpec = [
        provider.wallet.publicKey.toBuffer(),
        TOKEN_PROGRAM_ID.toBuffer(),
        tokenMint.toBuffer(),
        tokenAccount.toBuffer(),
        managerPK.publicKey.toBuffer(),
        tokenRecipient.toBuffer(),
    ]
    const userAllowance = await programAddress(allowanceSpec)
    const allowanceBytes = tokenAgent.account.tokenAllowance.size
    const allowanceRent = await provider.connection.getMinimumBalanceForRentExemption(allowanceBytes)
    console.log('User Allowance')
    console.log(userAllowance, allowanceBytes, allowanceRent)
    //console.log('Merchant PK')
    //console.log(merchantPK.toString())
    console.log('Manager')
    //console.log(managerPK.secretKey.toString())
    console.log('Manager Secret Key: ' + exportSecretKey(managerPK))

    if (false) {
        console.log('Create Allowance')
        await tokenAgent.rpc.createAllowance(
            true,                                       // Link token
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(allowanceBytes),              // Allowance size
            new anchor.BN(allowanceRent),               // Allowance rent
            new anchor.BN(1000 * 10000),                // Amount
            new anchor.BN(0),                           // Start time, or 0 for none
            new anchor.BN(0),                           // Expire time, or 0 for none
            {
                accounts: {
                    allowanceData: new PublicKey(userAllowance.pubkey),
                    userKey: provider.wallet.publicKey,
                    userAgent: new PublicKey(userAgent.pubkey),
                    delegateKey: managerPK.publicKey,
                    funderKey: provider.wallet.publicKey,
                    tokenMint: tokenMint,
                    tokenAccount: tokenAccount,
                    tokenProgram: TOKEN_PROGRAM_ID,
                    systemProgram: SystemProgram.programId,
                },
                remainingAccounts: [
                    { pubkey: tokenRecipient, isWritable: false, isSigner: false },
                ],
            }
        )
    }
    if (true) {
        console.log('Perform Delegated Transfer')
        await tokenAgent.rpc.delegatedTransfer(
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(50 * 10000),                  // Amount
            {
                signers: [managerPK],
                accounts: {
                    allowanceData: new PublicKey(userAllowance.pubkey),
                    userKey: provider.wallet.publicKey,
                    userAgent: new PublicKey(userAgent.pubkey),
                    userToken: tokenAccount,                            // From
                    tokenRecipient: tokenRecipient,                     // To
                    delegateKey: managerPK.publicKey,
                    tokenMint: tokenMint,
                    tokenProgram: TOKEN_PROGRAM_ID,
                },
            }
        )
    }
}

console.log('Begin')
main().then(() => console.log('Success')).catch(error => {
    console.log(error)
})
