A simple way to display your gen4 pokemon team in your streaming without having to worry about updating the overlay!

# How to use

The application can be used for **one person** use or you can connect with **a friend** to display both of your teams.

The only requirement is to download the application from the releases folder and the sprites you want in the sprites folder that gets generated with the execution of the application.

## Setting up your team

For this we have two modes. We can read the team from a save file or from text files.

- **Save file**: It is as simple as selecting the save file mode and then selecting the file. The application will listen for file changes and update the team when you save in-game.
- **Text files**: With the text files mode you can have different teams in the same execution of the application. Any .txt file that has `team` in its name will be read as a team. Updating the file will also automatically update the overlay

## Setting sprites to show

For copyright reasons the sprites are not in this repository.
When the application is first run, it will create a `sprites` folder.

It is as simple as putting any sprite you want in the sprites folder. When displaying a pokemon, it will get the image that has the name of the species of the pokemon (for example, if you have a pikachu in your team, it will get the image with name *pikachu* in the sprites)

There are some sources that have all the pokemon sprites with names, you can check in google and you will probably find one easily.

## Displaying the team in your streaming tool

For displaying your team, click the `Copy URL` button. This will copy to your clipboard an URL.
Put this URL in your streamer tool as a Web Source and that's all! You should be seeing your team in a 2x3 grid.

## Connecting with a friend.

To connect with a friend we use what we call _tickets_. This is in summary a unique text that identify your pc in the internet (if you want to know more about this, check the [iroh documentation](https://docs.iroh.computer/concepts/tickets))

1. Copy the ticket
2. Send your ticket to your friend
3. Your friend puts your ticket in his application
4. Now any team your friend has should be listed in your application!

Whenever your friend team is updated, it will updated in your application.

## Advanced: Modify the style of the grid

I have set the default of the application to be a grid of 2x3, but maybe your use case is different than mine.

You can overwrite the style in your streaming tool by putting custom CSS.

It would be as simple as modifying the container class properties. This would be an example if you wanted to have a 1x6 overlay instead:

```CSS
.team-grid {
	display: grid;
	grid-template-columns: repeat(6, 1fr);
	grid-template-rows: 1fr;
	gap: 30px;
	align-content: space-between;
}
```

I am pretty sure any AI can help you out with this. You can pass the whole [style file](./static/index.css) file to the AI and it will know what to do.

This is just an example, but you can also play with other things like changing the font of the names, changing the size of the sprites... etc :)

# About privacy note

If you are one of those kind of people that is worried about using an application that connects through internet with someone using a random application from github, don't worry about this one!

Currently how this application connects to your friend is using a peer-to-peer library that let's your friend and you connect without having any server in the middle.

There is only _one case_ in which you would use some kind of public server, and it is when some of you are behind a very restricting NAT. In that case, the handshake (starting the connection) is done through a public server. Once it is done, the connection is now peer-to-peer (you can see more about this in [the library documentaton](https://docs.iroh.computer/what-is-iroh))

# Technical details

Everything is developed using Rust, except for the frontend (the team overlay) that we serve a static HTML that connects to the server using a Websocket.

I have used the following crates for this application:

- **UI**: [iced](https://iced.rs/) (previously, I used egui and used Claude Opus to migrate to iced)
- **HTTP API**: [axum](https://github.com/tokio-rs/axum)
- **P2P Connection**: [iroh](https://www.iroh.computer/)

Expect some code not to be the best, as it has been ai-assisted generated and under the rush that this had to be running correctly for a tight deadline (don't hesitate to criticize anything from the code :) ).

# Roadmap

This section doesn't mean that these features are going to be implemented. This project was done by me for a soul-link locke competition I participated with some friends, so just expect some of these features to maybe be implemented by me if I ever participate in any other competition and I need any of these 😅.

- Currently the application only handles gen 4 save files. It would be nice to have other gens added too.
- Some kind of friends list so we don't have to pass the ticket in every execution (but this would require some kind of server between the two users and this would be against the peer-to-peer philosophy of this project, maybe make it optional for users to opt-in for this feature).

Feel free to contribute to the project!
