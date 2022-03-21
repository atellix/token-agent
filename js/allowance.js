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
    const tokenAccount = new PublicKey('AL95HoRjCnaWm2R8GFWoCdoFaPTDCqt7yrxQ69z4DCMD')
    const tokenRecipient = new PublicKey('3Qh11vYtpKamT8WK8mD5UgdeBCk3gP8Z5eh3oe98AsGC')

    var managerPK
    if (false) {
        managerPK = anchor.web3.Keypair.generate()
    } else {
        managerPK = importSecretKey('3h9y0bjxfj204gsf5rr9913hpkvwyvreatp65tc89mb7p7cm7wk250d9k60p3q9jgv65azenq853wfp1zztccbr5h4jyc97d7g13138')
    }

    const rootKey = await programAddress([tokenAgentPK.toBuffer()])
    let allowanceSpec = [
        tokenAccount.toBuffer(),
        managerPK.publicKey.toBuffer(),
    ]
    const userAllowance = await programAddress(allowanceSpec)
    console.log('User Allowance: ' + userAllowance['pubkey'])
    console.log('Manager Public Key: ' + managerPK.publicKey.toString())

    if (true) {
        console.log('Create Allowance')
        let tx = new anchor.web3.Transaction()
        tx.add(tokenAgent.transaction.createAllowance(
            true,                                       // Link token
            rootKey.nonce,                              // Root key nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(1000 * 10000),                // Amount
            new anchor.BN(0),                           // Start time, or 0 for none
            new anchor.BN(0),                           // Expire time, or 0 for none
            {
                accounts: {
                    allowanceData: new PublicKey(userAllowance.pubkey),
                    userKey: provider.wallet.publicKey,
                    rootKey: new PublicKey(rootKey.pubkey),
                    delegateKey: managerPK.publicKey,
                    tokenAccount: tokenAccount,
                    tokenProgram: TOKEN_PROGRAM_ID,
                    systemProgram: SystemProgram.programId,
                },
                //remainingAccounts: [],
            }
        ))
        //console.log(tx)
        console.log(await provider.send(tx))
    }
    if (false) {
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
    if (false) {
        console.log('Update Allowance')
        await tokenAgent.rpc.updateAllowance(
            true,                                       // Link token
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(1000 * 10000),                // Amount
            new anchor.BN(0),                           // Start time, or 0 for none
            new anchor.BN(0),                           // Expire time, or 0 for none
            {
                accounts: {
                    allowanceData: new PublicKey(userAllowance.pubkey),
                    userKey: provider.wallet.publicKey,
                    userAgent: new PublicKey(userAgent.pubkey),
                    delegateKey: managerPK.publicKey,
                    tokenMint: tokenMint,
                    tokenAccount: tokenAccount,
                    tokenProgram: TOKEN_PROGRAM_ID,
                },
            }
        )
    }
    if (false) {
        console.log('Perform Delegated Transfer 2')
        await tokenAgent.rpc.delegatedTransfer(
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(30 * 10000),                  // Amount
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
