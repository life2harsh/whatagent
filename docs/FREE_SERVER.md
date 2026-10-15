# Getting a free server

whatagent needs a small Linux computer that stays on all the time. Oracle Cloud gives you one for free, forever, on its "Always Free" tier. This page walks you through it, no Linux experience needed. It takes about 15 minutes.

whatagent is tiny: the smallest free server is plenty. It also doesn't need any ports opened, because it only makes outgoing connections.

## 1. Sign up

1. Go to [signup.cloud.oracle.com](https://signup.cloud.oracle.com) and create an account.
2. Oracle asks for a card to check you're a real person. Always Free resources don't cost anything, and you won't be charged unless you upgrade and use paid resources yourself.
3. Pick a **home region** close to you. You can't change it later, and free servers only run in your home region.

## 2. Create the server

1. Once you're signed in, open the menu (top left) > **Compute** > **Instances**, and click **Create instance**.
2. **Name:** anything, for example `whatagent`.
3. **Image:** click **Change image** (or **Edit** next to "Image and shape"), choose **Ubuntu**, and pick **22.04** or **24.04**.
4. **Shape:** click **Change shape**, choose **Ampere**, and pick **VM.Standard.A1.Flex** with 1 OCPU and 6 GB of memory. It's marked "Always Free-eligible".
   - If Oracle says it's **out of capacity**, try another "availability domain" (under Placement), try again later, or pick the AMD shape **VM.Standard.E2.1.Micro** instead. That one is also free and is enough for whatagent.
5. **Networking:** keep the defaults. If it asks, let it create a new virtual cloud network with a **public subnet**, and make sure **Assign a public IPv4 address** is on.
6. **SSH keys:** choose **Generate a key pair for me** and click **Download private key**. This file is how you log in to the server, so keep it safe and don't share it.
7. Click **Create**. After a minute or two the status turns **Running**. Copy the **Public IP address** shown on the page.

## 3. Connect to it

You'll type a few commands on your own computer. Replace `KEY` with the path to the key file you downloaded and `IP` with the public IP address.

**Windows** (10 or 11): open **PowerShell** from the Start menu and run:

```powershell
ssh -i "$env:USERPROFILE\Downloads\KEY" ubuntu@IP
```

**Mac or Linux:** open **Terminal** and run:

```bash
chmod 600 ~/Downloads/KEY
ssh -i ~/Downloads/KEY ubuntu@IP
```

The first time, it asks "Are you sure you want to continue connecting?". Type `yes` and press Enter. You're in when the prompt starts with `ubuntu@`.

## 4. Install whatagent

On the server, paste:

```bash
curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/install.sh | sudo bash
```

and answer its questions. The [README](../README.md#setting-it-up) explains each step. When it's done you can close the window: whatagent keeps running on the server, and starts again by itself if the server restarts.

## Good to know

- **Idle servers can be reclaimed.** Oracle may take back Always Free servers that sit nearly idle for a week, and whatagent is very light. To avoid that, you can upgrade the account to **Pay As You Go** (Billing > Upgrade and manage payment). Always Free resources stay free after upgrading; you only pay if you create paid things. Oracle's rules are on [this page](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier_topic-Always_Free_Resources.htm). If your server does get reclaimed, create a new one and run the installer again.
- **"UNPROTECTED PRIVATE KEY FILE" on Windows:** run this in PowerShell, then try `ssh` again:

  ```powershell
  icacls "$env:USERPROFILE\Downloads\KEY" /inheritance:r /grant:r "$($env:USERNAME):(R)"
  ```

- **"Permission denied (publickey)":** check the user is `ubuntu` and the `-i` path points to the private key you downloaded (not the `.pub` one).
- **Any other cheap VPS works too.** All you need is Ubuntu 22.04 or newer (or Debian 12 or newer), on x86 or ARM.
