# ESPuinocast - Update podcast playlists on an [ESPuino](https://github.com/biologist79/ESPuino)

![build workflow](https://github.com/Captain-Sandwich/espuinocast/actions/workflows/test-builds.yml/badge.svg)

## Idea

ESPuinocast is a standalone executable that reads a number of podcast feeds and produces an ESPuino-friendly m3u playlist.
That playlist is subsequently uploaded to an [ESPuino](https://github.com/biologist79/ESPuino).

## Configuration

The executable looks for a config file `config.ini` in its working directory. The configuration file format follows this example:


```ini
# The espuino block holds all configuration parameters for connecting to your espuino
[espuino]
host = espuino.local # default espuino.local; espuino host name or ip address
path = /podcasts # default: /podcasts; where to store playlists on the espuino, must exist already
proxy = http://localhost:8080 # optional; http proxy address. leave out for direct connection


# The names of all other config blocks start with 'podcast.'. The part after that is used as the
# playlist filename on the espuino.
# the following block produces the playlist '/podcasts/adventurezone.m3u' for example
[podcast.adventurezone]
url = https://feeds.simplecast.com/cYQVc__c # mandatory! URL of the podcast feed
reverse = true # reverse before truncating # optional. Reverse playlist, playing old episodes first
num = 16 # optional. truncate playlist to a maximum of 'num' entries.
file = adventurezone.m3u # optional: also write playlist to this local file

# EXAMPLE. The following block produces a playlist at '/podcasts/tagesthemen.m3u'
# which only includes the newest episode.
[podcast.tagesthemen]
url = https://www.tagesschau.de/multimedia/sendung/tagesthemen/podcast-tt-audio-100~podcast.xml
num = 1

```
