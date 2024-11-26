# Privastead

Privastead is a privacy-preserving home security camera solution that uses end-to-end encryption.
It has three key benefits:

* End-to-end encryption using the OpenMLS implementation of the Messaging Layer Security (MLS) protocol.
* Software-only solution that works with existing IP cameras with minimal trust assumptions about the IP camera.
* Rust implementation (camera hub, MLS code for the mobile app, and untrusted server).

## Components

The Privastead camera solution has three components:

* A camera hub, which runs on a local machine and directly interacts with IP camera(s).
* A mobile app that allows one to receive event notifications (e.g., motion) as well as livestream the camera remotely.
* An untrusted server that relays (encrypted) messages between the hub and the app. In addition, Privastead uses the Google Firebase Cloud Messaging (FCM) for notifications. Similar to the server, FCM is untrusted.

## Threat Model and Guarantees

The key advantage of the Privastead camera solution over existing home security camera solutions is that it provides strong privacy assurance using end-to-end encryption.
More specifically, it makes the following assumptions:

* It assumes that the local machine running the hub and the smartphone running the mobile app are secure and not compromised.
* It assumes that the server is fully untrusted and under the control of the adversary.
* It makes minimal trust assumptions about the IP camera. That is, it assumes that the camera does not have a covert, undisclosed network interface card (e.g., cellular) to connect to the Internet on its own (therefore, it's best that this is explicitly checked and verified by user). Other than that, the IP camera is untrusted and hence Privastead does not directly connect the camera to the Internet; rather, the camera is connected to the hub directly.

It then provides the following guarantees:

* It guarantees that only the hub and the mobile app have access to unecrypted videos.
* It guarantees that the server cannot decrypt the videos.
* It provides forward secrecy and post-comproise security through MLS (see definitions below).
* It does NOT currently hide the timing of events and livestreams from the adversary (who we assume is in control of the server and/or FCM channel).

Definitions: According to MLS: ``Forward secrecy means that messages sent at a certain point in time are secure in the face of later compromise of a group member. Post-compromise security means that messages are secure even if a group member was compromised at some point in the past.''
What do these mean in Privastead?
In Privastead, the camera hub and the mobile app are the only members in an MLS group used for transfer of videos.
They mean that if the key used to encrypt a video between the hub and the app is compromised, that key cannot be used to decrypt any of the videos sent before and after the compromised video.

## Supported Cameras

Privastead camera can theoretically support any IP camera (or any other camera that has an open interface).
The current prototype relies on RTSP and ONVIF support by the camera.
The former is used for streaming videos from the camera and the latter is used for querying events.
So far, the following cameras have been tested:

* Amcrest, model: IP4M-1041W ([Link](https://www.amazon.com/Amcrest-UltraHD-Security-4-Megapixel-IP4M-1041W/dp/B095XD17K5/) on Amazon)
    * Software Version: V2.800.00AC006.0.R, Build Date: 2023-10-27
    * WEB Version: V3.2.1.18144
    * ONVIF Version: 21.12(V3.1.0.1207744)

## Supported mobile OSes

* Android

## Tested smartphones (OS version)

* Google Pixel 8 Pro (Android 14)

## Tested execution environment for the hub

* Ubuntu (ffmpeg required)

## (Current) key limitations

* The app can pair with one camera only.
* The camera hub supports one camera only.
* The camera hub pairs with one app instance only.
* Performance may become a bottleneck for high camera resolutions and frame rates.

## Instructions

See [here](HOW_TO.md) for instructions for setting up Privastead.

## Mailing list

If you are interested in receiving email updates on progress on Privastead, sign up using this [form](https://forms.gle/ZNbTZ9QpaG1z9X2S6).

## Contributions

We welcome contributions to the project. Before working on a contribution, please check with us via email: privastead@gmail.com

Contributions are made under Privastead's [license](LICENSE).


## Project members

* Project Founder: Ardalan Amiri Sani (Ph.D., Computer Science professor at UC Irvine with expertise in computer security and privacy)

Note: this is a side project of Ardalan Amiri Sani, who works on it in his spare time.

## Disclaimers

This project uses cryptography libraries/software. Before using it, check your country's laws and regulations.