// Copyright The Mumble Developers. All rights reserved.
// Use of this source code is governed by a BSD-style license
// that can be found in the LICENSE file at the root of the
// Mumble source tree or at <https://www.mumble.info/LICENSE>.

#ifndef MUMBLE_MUMBLE_WEBSOCKETCONNECTION_H_
#define MUMBLE_MUMBLE_WEBSOCKETCONNECTION_H_

#ifdef USE_WEBSOCKET

#	include "MumbleProtocol.h"
#	include "crypto/CryptState.h"
#	include "crypto/CryptStateOCB2.h"

#	include <QtCore/QByteArray>
#	include <QtCore/QObject>
#	include <QtCore/QUrl>
#	include <QtNetwork/QHostAddress>
#	include <QtNetwork/QSslCertificate>
#	include <QtNetwork/QSslCipher>
#	include <QtNetwork/QSslError>
#	include <QtWebSockets/QWebSocket>

#	include <memory>

namespace google {
namespace protobuf {
	class Message;
}
} // namespace google

/**
 * WebSocketConnection provides the same interface as Connection but uses
 * QWebSocket as its underlying transport.  The Mumble binary protocol frames
 * are carried as binary WebSocket messages, preserving the same 2-byte type +
 * 4-byte length + payload layout used over TCP/TLS.
 */
class WebSocketConnection : public QObject {
private:
	Q_OBJECT
	Q_DISABLE_COPY(WebSocketConnection)

	QWebSocket *m_socket;
	QByteArray m_readBuffer;
	Mumble::Protocol::TCPMessageType m_type;
	int m_packetLength;

protected slots:
	void onBinaryMessageReceived(const QByteArray &message);
	void onError(QAbstractSocket::SocketError error);
	void onDisconnected();
	void onSslErrors(const QList< QSslError > &errors);

public:
	WebSocketConnection(QObject *parent, QWebSocket *socket);
	~WebSocketConnection();

	static void messageToNetwork(const ::google::protobuf::Message &msg, Mumble::Protocol::TCPMessageType msgType,
								 QByteArray &cache);

	void sendMessage(const ::google::protobuf::Message &msg, Mumble::Protocol::TCPMessageType msgType,
					 QByteArray &cache);
	void sendMessage(const QByteArray &qbaMsg);
	void disconnectSocket(bool force = false);
	void forceFlush();

	QList< QSslCertificate > peerCertificateChain() const;
	QSslCipher sessionCipher() const;
	QHostAddress peerAddress() const;
	quint16 peerPort() const;
	QHostAddress localAddress() const;
	quint16 localPort() const;

	bool bDisconnectedEmitted;

	std::unique_ptr< CryptState > csCrypt;

signals:
	void connectionClosed(QAbstractSocket::SocketError, const QString &reason);
	void message(Mumble::Protocol::TCPMessageType type, const QByteArray &);
	void handleSslErrors(const QList< QSslError > &);

public slots:
	void proceedAnyway();
};

#endif // USE_WEBSOCKET

#endif // MUMBLE_MUMBLE_WEBSOCKETCONNECTION_H_
