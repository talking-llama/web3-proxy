mod common;

use crate::common::TestApp;
use ethers::prelude::Signer;
use ethers::types::Signature;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, info, trace, warn};
use web3_proxy::frontend::admin::AdminIncreaseBalancePost;
use web3_proxy::frontend::users::authentication::{LoginPostResponse, PostLogin};

// #[cfg_attr(not(feature = "tests-needing-docker"), ignore)]
#[ignore = "under construction"]
#[test_log::test(tokio::test)]
async fn test_admin_imitate_user() {
    let x = TestApp::spawn(true).await;

    todo!();
}

#[cfg_attr(not(feature = "tests-needing-docker"), ignore)]
#[test_log::test(tokio::test)]
async fn test_admin_grant_credits() {
    let x = TestApp::spawn(true).await;
    let r = reqwest::Client::new();

    // Setup variables that will be used
    let login_post_url = format!("{}user/login", x.proxy_provider.url());
    let increase_balance_post_url = format!("{}admin/increase_balance", x.proxy_provider.url());

    // TODO: I should make a wallet an admin wallet first ...
    let admin_wallet = x.wallet(1);
    let user_wallet = x.wallet(2);

    // Login the admin
    let admin_login_get_url = format!(
        "{}user/login/{:?}",
        x.proxy_provider.url(),
        admin_wallet.address()
    );
    let admin_login_message = r.get(admin_login_get_url).send().await.unwrap();
    let admin_login_message = admin_login_message.text().await.unwrap();
    // Sign the message and POST it to login as admin
    let admin_signed: Signature = admin_wallet
        .sign_message(&admin_login_message)
        .await
        .unwrap();
    debug!(?admin_signed);
    let admin_post_login_data = PostLogin {
        msg: admin_login_message,
        sig: admin_signed.to_string(),
        referral_code: None,
    };
    debug!(?admin_post_login_data);
    let admin_login_response = r
        .post(&login_post_url)
        .json(&admin_post_login_data)
        .send()
        .await
        .unwrap()
        .json::<LoginPostResponse>()
        .await
        .unwrap();
    debug!(?admin_login_response);

    // Also login the user (to create the user)
    let user_login_get_url = format!(
        "{}user/login/{:?}",
        x.proxy_provider.url(),
        user_wallet.address()
    );
    let user_login_message = r.get(user_login_get_url).send().await.unwrap();
    let user_login_message = user_login_message.text().await.unwrap();

    // Sign the message and POST it to login as admin
    let user_signed: Signature = user_wallet.sign_message(&user_login_message).await.unwrap();
    debug!(?user_signed);
    let user_post_login_data = PostLogin {
        msg: user_login_message,
        sig: user_signed.to_string(),
        referral_code: None,
    };
    debug!(?user_post_login_data);
    let user_login_response = r
        .post(login_post_url)
        .json(&user_post_login_data)
        .send()
        .await
        .unwrap()
        .json::<LoginPostResponse>()
        .await
        .unwrap();
    debug!(?user_login_response);

    // Now I need to make the user an admin ...

    // Login the user
    // Use the bearer token of admin to increase user balance
    let increase_balance_data = AdminIncreaseBalancePost {
        user_address: user_wallet.address(), // set user address to increase balance
        amount: Decimal::from(100),          // set amount to increase
        note: Some("Test increasing balance".to_string()),
    };
    let increase_balance_response = r
        .post(increase_balance_post_url)
        .json(&increase_balance_data)
        .bearer_auth(admin_login_response.bearer_token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();

    debug!(?increase_balance_response);

    // Check if the response is as expected
    // assert_eq!(increase_balance_response["user"], user_wallet.address());
    assert_eq!(
        Decimal::from_str(increase_balance_response["amount"].as_str().unwrap()).unwrap(),
        Decimal::from(100)
    );

    x.wait().await;
}

// #[cfg_attr(not(feature = "tests-needing-docker"), ignore)]
#[ignore = "under construction"]
#[test_log::test(tokio::test)]
async fn test_admin_change_user_tier() {
    let x = TestApp::spawn(true).await;
    todo!();
}
