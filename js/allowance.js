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
        managerPK = anchor.web3.Keypair.fromSecretKey(new Uint8Array([119,207,69,14,225,84,57,222,85,25,143,51,99,176,218,91,113,19,12,59,193,78,243,206,184,85,38,75,183,145,39,62,146,87,67,118,8,41,7,116,10,101,211,14,240,90,17,102,73,222,131,47,70,195,116,127,49,161,11,49,40,205,2,34]))
    }
    //const managerAP = anchor.web3.Keypair.generate()

    const userAgent = await programAddress([provider.wallet.publicKey.toBuffer()])
    const userAllowance = await programAddress([
        provider.wallet.publicKey.toBuffer(),
        TOKEN_PROGRAM_ID.toBuffer(),
        tokenMint.toBuffer(),
        tokenAccount.toBuffer(),
        managerPK.publicKey.toBuffer(),
    ])
    const allowanceBytes = tokenAgent.account.tokenAllowance.size
    const allowanceRent = await provider.connection.getMinimumBalanceForRentExemption(allowanceBytes)
    console.log('User Allowance')
    console.log(userAllowance, allowanceBytes, allowanceRent)
    //console.log('Merchant PK')
    //console.log(merchantPK.toString())
    console.log('Manager')
    console.log(managerPK.secretKey.toString())

    if (true) {
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
        console.log('Create Allowance')
        await tokenAgent.rpc.createAllowance(
            true,                                       // Link token
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(allowanceBytes),              // Allowance size
            new anchor.BN(allowanceRent),               // Allowance rent
            new anchor.BN(1000 * 1000000),              // Amount
            new anchor.BN(0),                           // Start time, or 0 for none
            new anchor.BN(0),                           // Expire time, or 0 for none
            {
                accounts: {
                    userKey: provider.wallet.publicKey,
                    userAgent: new PublicKey(userAgent.pubkey),
                    delegateKey: managerPK.publicKey,
                    tokenMint: tokenMint,
                    tokenAccount: tokenAccount,
                    tokenProgram: TOKEN_PROGRAM_ID,
                },
                remainingAccounts: [
                    { pubkey: provider.wallet.publicKey, isWritable: true, isSigner: true },
                    { pubkey: new PublicKey(userAllowance.pubkey), isWritable: true, isSigner: false },
                    { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
                ],
            }
        )
    }
    if (false) {
        console.log('Perform Delegated Transfer')
        await tokenAgent.rpc.delegatedTransfer(
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(500 * 1000000),               // Amount
            {
                signers: [managerPK],
                accounts: {
                    allowanceData: new PublicKey(userAllowance.pubkey),
                    userKey: provider.wallet.publicKey,
                    userAgent: new PublicKey(userAgent.pubkey),
                    userToken: tokenAccount,                            // From
                    tokenRecipient: new PublicKey(merchantTK.pubkey),   // To
                    delegateKey: managerPK.publicKey,
                    tokenMint: tokenMint,
                    tokenProgram: TOKEN_PROGRAM_ID,
                },
            }
        )
    }
    if (true) {
        console.log('Update Allowance')
        await tokenAgent.rpc.updateAllowance(
            true,                                       // Link token
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(1000 * 1000000),              // Amount
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
    if (true) {
        console.log('Perform Delegated Transfer 2')
        await tokenAgent.rpc.delegatedTransfer(
            userAgent.nonce,                            // User agent nonce
            userAllowance.nonce,                        // Allowance nonce
            new anchor.BN(500 * 1000000),               // Amount
            {
                signers: [managerPK],
                accounts: {
                    allowanceData: new PublicKey(userAllowance.pubkey),
                    userKey: provider.wallet.publicKey,
                    userAgent: new PublicKey(userAgent.pubkey),
                    userToken: tokenAccount,                            // From
                    tokenRecipient: new PublicKey(merchantTK.pubkey),   // To
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
